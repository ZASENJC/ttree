# AGENTS.md

This file provides guidance to Codex (Codex.ai/code) when working with code in this repository.

## Project Overview

ttree is a lightweight macOS AI translator built with **Tauri 2 + Svelte 5 + Rust**. It is designed as a hidden spotlight-style app: the main window starts hidden, global shortcuts show it instantly, and the window auto-hides on blur.

Core features:
- `CmdOrCtrl+Shift+Space` by default toggles the translation window.
- `CmdOrCtrl+Shift+S` by default triggers screenshot OCR translation.
- Translation engines: Google and Bing free web endpoints (no key) plus OpenAI-compatible streaming chat completions. A 中⇄英 "adaptive" target auto-picks direction by script detection.
- macOS OCR uses Apple Vision via `objc2-vision` (`VNRecognizeTextRequest`).
- Settings persist locally through `tauri-plugin-store`; shortcuts are user-configurable and re-registered immediately after saving. API keys are stored in the macOS Keychain, not the plaintext store.

## Common Commands

Install dependencies:

```bash
npm install
```

If npm blocks native install scripts in this environment, approve the packages already listed under `allowScripts` in `package.json`:

```bash
npm approve-scripts esbuild fsevents
```

Run the app in development mode:

```bash
npm run tauri dev
```

The Tauri window is configured with `visible: false`, so the app may appear to launch with no window. Use the configured global shortcut (default `CmdOrCtrl+Shift+Space`) to show it.

Frontend-only development server:

```bash
npm run dev
```

Frontend checks and build:

```bash
npm run check
npm run build
```

Frontend tests:

```bash
npm test
```

Rust backend checks/tests:

```bash
cd src-tauri && cargo check
cd src-tauri && cargo test
cd src-tauri && cargo test <test_name>
cd src-tauri && cargo clippy
cd src-tauri && cargo fmt
```

Production app build:

```bash
npm run tauri build
```

Build outputs are under `src-tauri/target/release/bundle/`, including `bundle/macos/TTREE.app` and `bundle/dmg/*.dmg` (productName is `TTREE`).

An ordinary local `tauri build` is not proof that a distributable macOS build is valid. Before release, the final `.app`, DMG, updater archive, updater signature, and `latest.json` must pass the community-signing checks described below.

## Release Process (应用内更新)

The app self-updates via the Tauri Updater plugin. Releases are built and signed by CI, then published to GitHub Releases; installed apps fetch `latest.json` from the repo and update in place.

**When the user asks to publish a new version, follow these steps in order:**

1. **Bump the version in both places** (they must match) and commit:
   - `package.json` → `"version"`
   - `src-tauri/tauri.conf.json` → `"version"`
2. **Verify locally** before tagging:
   ```bash
   npm run check && npm test
   cd src-tauri && cargo check && cd ..
   ```
3. **Tag and push the tag** (triggers the release workflow on `v*` tags):
   ```bash
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```
4. CI (`.github/workflows/release.yml`) builds a single `universal-apple-darwin` (arm64 + x86_64 combined; the runner only ships arm64 by default, so the workflow explicitly runs `rustup target add x86_64-apple-darwin` first). `scripts/package-macos-community.sh` signs the app with the persistent community certificate, rebuilds the DMG/updater archive, and verifies both containers.
5. The workflow creates a draft Release first. It must pass `scripts/verify-macos-community-signature.sh` and verify all four release assets before publishing. Manual `workflow_dispatch` runs are verification-only and never publish.
6. Watch the run: `gh run watch -R ZASENJC/ttree` (or check https://github.com/ZASENJC/ttree/actions). Confirm the published Release lists `latest.json` + the signed `.tar.gz`/`.sig` assets.
7. Installed apps auto-check for updates on startup and every 6h, hitting the network only once ≥7 days since the last check; users install from 设置 → 通用. New installs get the latest Release immediately.

**Updater-signing prerequisites (already done, do not repeat):** signing keys live at `~/.config/ttree-updater/` on this machine; the public key is in `tauri.conf.json` and the private key is in the `TAURI_SIGNING_PRIVATE_KEY` repo secret. The private key **must carry a strong passphrase**: without a second factor, anyone who exfiltrates the key can sign an updater package that installed apps auto-accept (RCE). The passphrase lives in macOS Keychain (account `updater-signing-password`, service `com.samwstu.ttree`) for local builds and in the `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` repo secret for CI. To build a signed update locally instead of via CI: `source ./scripts/signing-env.sh && npm run tauri build` — the script reads the passphrase from Keychain automatically. If the key was ever stored unencrypted, rotate it: generate a new minisign keypair with a passphrase, update `tauri.conf.json` pubkey, and ship one release on the old key that bumps users to the new key.

**Community code-signing prerequisites (configured July 2026):** the persistent certificate is `TTREE Open Source Code Signing`, SHA-1 `B8D254561BB6CAD78AA71FA18D62A32D10F50C08`. Local encrypted material lives under `~/.config/ttree-code-signing/`; its password is stored in Keychain account `community-code-signing-password`, service `com.samwstu.ttree`. CI uses `MACOS_COMMUNITY_CERTIFICATE`, `MACOS_COMMUNITY_CERTIFICATE_PASSWORD`, `MACOS_COMMUNITY_SIGNING_IDENTITY`, and `MACOS_COMMUNITY_CERT_SHA1`. Never commit, print, rotate, recreate, or replace the certificate/private key. Losing or rotating it changes the TCC identity and forces every user to reauthorize Screen Recording.

### macOS Code Signing and Screen Recording TCC

This is a release-critical invariant. macOS Screen Recording permission is attached to the app's code signing designated requirement, not just its display name, path, or `CFBundleIdentifier`.

- Screenshot OCR must keep using Apple's native free-selection command `/usr/sbin/screencapture -i -x -r`, followed by Apple Vision OCR. Do not replace it with a custom Tauri selection window, CoreGraphics overlay, or a different screenshot architecture merely to address a permission prompt.
- The failure mode seen in July 2026 was an unsigned release bundle whose executable only had a linker-generated ad-hoc signature. Its identity looked like `Identifier=ttree-<hash>`, `TeamIdentifier=not set`, `Info.plist=not bound`, and `designated => cdhash ...`. Every rebuild changed the CDHash, so System Settings could show TTREE as enabled for the old build while the new build was denied and asked again.
- Tauri updater minisign keys and macOS code-signing certificates solve different problems. A valid `.tar.gz.sig` does not give the `.app` a stable macOS identity and does not preserve TCC permissions; both signing layers are required.
- The current GitHub release mode is community-signed, not Apple-notarized. Every distributed build must use the same `TTREE Open Source Code Signing` certificate and the explicit designated requirement `identifier "com.samwstu.ttree" and certificate leaf = H"b8d254561bb6cad78aa71fa18d62a32d10f50c08"`.
- Never publish when `codesign --verify --deep --strict` fails, `Signature=adhoc`, the authority/fingerprint differs, `Info.plist` is not bound, sealed resources are absent, or the designated requirement is identifier-only/`cdhash`-based. Run `scripts/verify-macos-community-signature.sh` as the hard gate.
- Gatekeeper rejection by `spctl` is expected because the community certificate is not Apple-notarized. Release notes and README must instruct first-time users to use System Settings → Privacy & Security → Open Anyway, then grant Screen & System Audio Recording and restart TTREE.
- `workflow_dispatch` must build/upload temporary verification artifacts only. Only a pushed `v*` tag matching `package.json` may create a draft Release, and the draft may publish only after asset verification.
- A development build working is insufficient. For permission regression testing, install packaged build A, authorize Screen Recording once, then create packaged build B with a different CDHash but the same certificate-backed designated requirement. Install B without calling `tccutil reset`; OCR must work without another permission prompt.
- Migrating users from an old ad-hoc build to the first community-signed build may require removing/resetting the stale TTREE Screen Recording entry and authorizing once. Later builds signed by the same identity must retain permission.
- A custom ad-hoc requirement may be used only for contained local diagnosis. It is not secure against bundle-ID impersonation and must never be distributed or used as the production fix.
- If the project later joins Apple Developer Program, Developer ID + notarization may replace community signing, but that is an intentional identity migration requiring its own user reauthorization plan. Do not silently mix or switch signing modes.

## Architecture

### Tauri / Rust backend

`src-tauri/src/lib.rs` is the backend composition point. It registers plugins, commands, global shortcuts, tray setup, macOS vibrancy, and the blur-to-hide window behavior.

Important backend modules:

- `commands.rs` exposes Tauri commands used by the Svelte frontend:
  - translation (Google / Bing one-shot, or OpenAI streaming)
  - AI chat (`chat`, `get/set_chat_ai_config`) — separate config from translation
  - screenshot OCR（Apple 原生 `screencapture` 自由框选 + Vision OCR）
  - show main window
  - get/set OpenAI translation config, get/set appearance config
  - get/set shortcut config, `set_shortcut_recording` (suppresses global shortcuts during key capture)
  - `get_pinned` / `set_pinned` — window always-on-top + blur-no-hide toggle
  - chat history: `load_chat_history`, `load_conversations`, `start_new_conversation`, `append_chat_history`, `clear_chat_history`
- `shortcut.rs` builds the global shortcut plugin and dispatches five registered shortcuts: `toggle` (show/hide window, emits `trigger-translate` on show to reset the frontend to the translate view), `ocr` (screenshot OCR), `ai_dialog` (open AI chat), `selection_translate` (capture selected text and translate), and `selection_ai_dialog` (capture selected text and send to AI). All are optional. Two pairs may intentionally share the same key — `toggle`↔`selection_translate` and `ai_dialog`↔`selection_ai_dialog`; on collision the selection action wins (dispatch order in `ParsedShortcuts::entries` lists selection actions first), so a shared key runs the selection behavior when text is selected and falls back to the plain toggle/dialog otherwise. All other key collisions are rejected at validation. Startup uses persisted shortcut config; saving validates, unregisters existing shortcuts, and registers the deduplicated set immediately.
- `config.rs` owns persisted config models (`OpenAiConfig`, `ChatAiConfig`, `ShortcutConfig`, `AppearanceConfig`) and store reads/writes through `tauri-plugin-store`. Translation and chat use separate AI config entries; each carries its own optional prompt (`translate_prompt`, `chat_prompt`). `ShortcutConfig` holds five independently-optional shortcuts: `toggle`, `ocr`, `ai_dialog`, `selection_translate`, `selection_ai_dialog` (empty string disables one). **API keys never touch the store** — `config` reads/writes them through `keychain`. `save_openai`/`save_chat_ai` validate `base_url` (must be http(s), rejects `file:`/`data:`) and after a successful Keychain write they also delete any leftover legacy plaintext `api_key` key from the store. `Debug` impls redact `api_key`. Window size is persisted too, but debounced (500ms) and written on a blocking thread because `WindowEvent::Resized` fires dozens of times/second during a drag.
- `keychain.rs` wraps macOS Keychain generic-password items (`security-framework`). Service is the bundle id `com.samwstu.ttree`; accounts are `openai.api_key` / `chat_ai.api_key`. `get_secret` returns `Ok(None)` for a missing item (first-use); non-macOS is a compile-gated stub that errors. `config::load_secret_migrated` does a one-time plaintext→Keychain migration: if the Keychain has nothing but the old store key still holds a plaintext value, it copies the value into the Keychain and clears the store copy.
- `history.rs` persists AI chat history as line-delimited JSON (`chat-history.jsonl`, a separate file from `settings.json`). Append-only writes (one line per message, single `write_all` per round), a `static Mutex` serializes all read/append/clear, and a single corrupt line is skipped rather than aborting the parse. Conversations are split by `{"type":"session_start"}` markers (`#[serde(untagged)]` so legacy `{"role","content"}` lines still parse); `Conversation.summary` is the first user line truncated to 40 chars. `read_conversations` returns old→new; `read_all` flattens to a message stream for backward compatibility.
- `window.rs` centralizes main-window show/hide/focus behavior. The app keeps the window hidden instead of destroying it for instant recall.
- `translate/mod.rs` dispatches by `Engine` (`openai`/`google`/`bing`), owns a shared pooled `reqwest::Client` (two instances: default, and one with a cookie jar for Bing's token dance), and disables redirects (SSRF guard — OpenAI-compatible POSTs are single-hop, so 3xx can't bounce an `api_key`-bearing request to an internal host). It also defines `ChatMessage` and `validate_messages` — the IPC-boundary guard (role whitelist `user`/`assistant`/`system`, 64 KiB per message, ≤200 messages) applied to both `chat` and `append_chat_history`.
- `translate/google.rs` calls the free `translate.googleapis.com` endpoint and parses its nested array response.
- `translate/bing.rs` calls the free `bing.com/ttranslatev3` endpoint. Bing needs a one-shot token (IG / IID / key / token) scraped from the translator page's HTML before POSTing, so it uses the cookie-store client and a browser User-Agent. Unofficial endpoint — may break or rate-limit.
- `translate/openai.rs` streams OpenAI-compatible SSE responses and emits `translate-chunk` (translation) or `chat-chunk` (AI chat) events to the frontend.
- `selection/macos.rs` captures the currently selected text via the macOS Accessibility API; falls back to a temporary `Cmd+C` + clipboard read if Accessibility fails. Non-macOS returns `None`.
- `screenshot.rs` invokes `/usr/sbin/screencapture -i -x -r` for Apple's native interactive free-selection UI, writes the selected region to a unique temporary PNG, and treats Esc/no file as cancellation. `commands::screenshot_ocr` sends that PNG to Vision and removes it afterward.
- `ocr/vision.rs` performs macOS-native OCR with Apple Vision (`VNRecognizeTextRequest`).
- `tray.rs` creates the menu bar tray menu (`翻译`, `设置`, `退出`).

### Svelte frontend

`src/App.svelte` holds two pieces of main-window view state: `view` (`main` / `settings`) and, within `main`, `mode` (`translate` / `chat`). On mount it loads chat history + appearance config, reads the current version, and kicks off the update auto-check. It listens for five backend events: `open-settings`, `trigger-translate`, `trigger-ocr`, `trigger-selected-translate`, `trigger-ai-dialog`. Screenshot selection is owned by the native macOS service; there is no screenshot-selection frontend window.

Important frontend areas:

- `src/lib/components/TranslatePanel.svelte` is the main spotlight UI. It handles Enter-to-translate, Esc-to-hide, engine/language selectors, and the dual-column result view (web translation + AI translation shown side by side).
- `src/lib/views/ChatPanel.svelte` is the AI chat view. It sends multi-turn messages, streams the assistant reply via `chat-chunk`, and surfaces the conversation-history sidebar.
- `src/lib/stores/translation.svelte.ts` owns translation state, including two web engines (`google`/`bing`) switchable via `WEB_ENGINES` and an adaptive target (default) that calls `resolveAdaptiveTarget` — CJK-containing input → English, otherwise → Chinese.
- `src/lib/stores/chat.svelte.ts` owns chat state and orchestrates history. `sendText`/`reset` drive the AI-dialog flow; `recentChatContext` (from `chatContext.ts`) trims to the last 24 messages and drops leading orphan assistant turns before sending.
- `src/lib/stores/appearance.svelte.ts` applies panel opacity to a `--panel-opacity` CSS var on `document.documentElement`. The opacity min (0.35) is intentionally kept in sync with `config.rs::PANEL_OPACITY_MIN` so frontend/backend clamping agree.
- `src/lib/stores/update.svelte.ts` drives the in-app updater. It checks on startup if ≥7 days since the last check (persisted in `settings.json` as `update.last_check`), re-checks every 6h while running, and only re-stamps the timestamp on `uptodate`/`error` (so an available-but-uninstalled update survives). Download → install → `relaunch()`.
- `src/lib/stores/languages.ts` holds the language list, the `adaptive` 中⇄英 target, and `resolveAdaptiveTarget` (script-based).
- The `.svelte.ts` stores (`translation`, `chat`, `appearance`, `update`) use the `.svelte.ts` extension because they contain Svelte 5 runes (`$state`). Do not move rune-based stores back to plain `.ts`; doing so leaves `$state` uncompiled and causes a runtime white screen. (`chatContext.ts` / `languages.ts` / `shortcuts.ts` are plain modules with no runes.)
- `src/lib/shortcuts/shortcutRecorder.ts` is a pure utility that converts keyboard events into Tauri-style shortcut strings. It has a Vitest unit test (`shortcutRecorder.test.ts`).
- `src/lib/api/tauri.ts` is the frontend boundary for all `invoke` calls and event subscriptions. `Engine` here is the string union `"openai" | "google" | "bing"`, matching the Rust `Engine` enum's `rename_all = "lowercase"`.
- `src/lib/views/Settings.svelte` edits OpenAI config, chat config, custom shortcuts, autostart, panel opacity, and exposes the updater (check/download/install). It temporarily pins the window on enter and restores the prior pin state on leave.
- Styling is based on CSS custom properties in `src/styles/tokens.css` plus global rules in `src/styles/global.css`.

## Runtime Notes

- Screenshot OCR requires macOS Screen Recording permission. In dev mode the permission prompt may refer to the terminal or dev binary, so dev success does not validate a release package. Packaged builds must keep the certificate-pinned community designated requirement unchanged across updates. Selected-text capture is separate: it prefers Accessibility permission and falls back to a temporary copy.
- OpenAI translation results stream via the `translate-chunk` event; AI chat streams via `chat-chunk`; Google and Bing return a complete string from the `translate` command.
- When the window is pinned (`set_pinned`), it stays always-on-top and does not auto-hide on blur.
- The Google and Bing endpoints are unofficial free endpoints and may be rate-limited or break (Bing scrapes a token from HTML, which is especially fragile).
- API keys live in the macOS Keychain, not the plaintext store. The store holds only non-sensitive config; the only trace of keys is transiently during the one-time plaintext→Keychain migration.

## Current Caveats

- The README may lag behind implementation details; check `config.rs`, `shortcut.rs`, and `Settings.svelte` for the current shortcut behavior. (It also still claims API keys are plaintext and only two engines exist — both outdated; trust the code.)
- Rust unit tests cover: native screenshot command flags, translation response/token parsing (Google, Bing), SSE parsing (OpenAI), selection text normalization, message validation, chat-history JSONL parsing + summary truncation + conversation splitting, and base_url normalization. `scripts/tests/verify-macos-community-signature.test.sh` and `scripts/tests/release-workflow.test.sh` cover the release identity/workflow contract. Frontend unit tests cover the shortcut recorder (`shortcutRecorder.test.ts`), chat-context trimming (`chatContext.test.ts`), and the language list / adaptive target (`languages.test.ts`); no component-level test suite yet.
