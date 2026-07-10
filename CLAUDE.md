# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

ttree is a lightweight macOS AI translator built with **Tauri 2 + Svelte 5 + Rust**. It is designed as a hidden spotlight-style app: the main window starts hidden, global shortcuts show it instantly, and the window auto-hides on blur.

Core features:
- `CmdOrCtrl+Shift+Space` by default toggles the translation window.
- `CmdOrCtrl+Shift+S` by default triggers screenshot OCR translation.
- Translation engines: Google and Bing free web endpoints (no key) plus OpenAI-compatible streaming chat completions. A 中⇄英 "adaptive" target auto-picks direction by script detection.
- macOS OCR uses Apple Vision via `objc2-vision` (`VNRecognizeTextRequest`).
- Settings persist locally through `tauri-plugin-store`; shortcuts are user-configurable and re-registered immediately after saving. API keys are stored in the macOS Keychain, not the plaintext store.

## Common Commands

Install dependencies from the lockfile without running arbitrary lifecycle scripts:

```bash
npm run ci:install
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

An ordinary local `tauri build` is only a development artifact. It is not proof that a distributable macOS build has the fixed community identity, valid updater signature, or verified release containers.

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
4. CI (`.github/workflows/release.yml`) builds `universal-apple-darwin` without signing material, then imports the fixed community certificate only for the signing/packaging stage. `scripts/package-macos-community.sh` signs the app, rebuilds the DMG/updater archive, and verifies both containers.
5. The workflow creates a draft Release first. It downloads the four actual draft assets and re-verifies their checksums, updater signature, manifest, and pinned community identity before publishing. Manual `workflow_dispatch` runs are verification-only and never publish.
6. Watch the run: `gh run watch -R ZASENJC/ttree` (or check https://github.com/ZASENJC/ttree/actions). Confirm the published Release lists `latest.json` plus the signed `.tar.gz`/`.sig` assets.
7. Installed apps auto-check for updates on startup and every 6h, hitting the network only once ≥7 days since the last check; users install from 设置 → 通用. New installs get the latest Release immediately.

**Updater-signing prerequisites (already done, do not repeat):** signing keys live at `~/.config/ttree-updater/`; the public key is in `tauri.conf.json`, and CI stores the encrypted private key plus its strong passphrase in separate secrets. For a local signed package, run `source ./scripts/signing-env.sh && ./scripts/package-macos-community.sh`; the package script withholds the updater key from npm/Cargo and exposes it only to the signer subprocess.

**Community code-signing invariant:** every distributed build must use `TTREE Open Source Code Signing`, SHA-1 `B8D254561BB6CAD78AA71FA18D62A32D10F50C08`, with designated requirement `identifier "com.samwstu.ttree" and certificate leaf = H"b8d254561bb6cad78aa71fa18d62a32d10f50c08"`. Never rotate, recreate, print, or replace that identity. It preserves macOS Screen Recording TCC authorization across updates. These releases are community-signed, not Apple-notarized; Gatekeeper's first-open warning is expected.

## Architecture

### Tauri / Rust backend

`src-tauri/src/lib.rs` is the backend composition point. It registers plugins, commands, global shortcuts, tray setup, macOS vibrancy, and the blur-to-hide window behavior.

Important backend modules:

- `commands.rs` exposes Tauri commands used by the Svelte frontend:
  - translation (Google / Bing one-shot, or OpenAI streaming)
  - AI chat (`chat`, `get/set_chat_ai_config`) — separate config from translation
  - screenshot OCR（Apple 原生自由框选 + Vision OCR）
  - show main window
  - get/set OpenAI translation config, get/set appearance config
  - get/set shortcut config, `set_shortcut_recording` (suppresses global shortcuts during key capture)
  - `get_pinned` / `set_pinned` — window always-on-top + blur-no-hide toggle
  - chat history: `load_chat_history`, `load_conversations`, `start_new_conversation`, `append_chat_history`, `clear_chat_history`
- `shortcut.rs` builds the global shortcut plugin and dispatches five registered shortcuts: `toggle` (show/hide window, emits `trigger-translate` on show to reset the frontend to the translate view), `ocr` (screenshot OCR), `ai_dialog` (open AI chat), `selection_translate` (capture selected text and translate), and `selection_ai_dialog` (capture selected text and send to AI). All are optional. Two pairs may intentionally share the same key — `toggle`↔`selection_translate` and `ai_dialog`↔`selection_ai_dialog`; on collision the selection action wins (dispatch order in `ParsedShortcuts::entries` lists selection actions first), so a shared key runs the selection behavior when text is selected and falls back to the plain toggle/dialog otherwise. All other key collisions are rejected at validation. Startup uses persisted shortcut config; saving validates, unregisters existing shortcuts, and registers the deduplicated set immediately.
- `config.rs` owns persisted config models (`OpenAiConfig`, `ChatAiConfig`, `ShortcutConfig`, `AppearanceConfig`) and store reads/writes through `tauri-plugin-store`. Translation and chat use separate AI config entries; each carries its own optional prompt (`translate_prompt`, `chat_prompt`). `ShortcutConfig` holds five independently-optional shortcuts: `toggle`, `ocr`, `ai_dialog`, `selection_translate`, `selection_ai_dialog` (empty string disables one). **API keys never touch the store** — `config` reads/writes endpoint-bound records through `keychain`, never returns real keys to the WebView, requires HTTPS for remote endpoints (HTTP is loopback-only), and compensates Store/Keychain failures. `Debug` impls redact `api_key`. Window size is persisted too, but debounced (500ms) and written on a blocking thread because `WindowEvent::Resized` fires dozens of times/second during a drag.
- `keychain.rs` wraps macOS Keychain generic-password items (`security-framework`). Service is the bundle id `com.samwstu.ttree`; accounts are `openai.api_key` / `chat_ai.api_key`. `get_secret` returns `Ok(None)` for a missing item (first-use); non-macOS is a compile-gated stub that errors. `config::load_secret_migrated` does a one-time plaintext→Keychain migration: if the Keychain has nothing but the old store key still holds a plaintext value, it copies the value into the Keychain and clears the store copy.
- `history.rs` persists AI chat history as line-delimited JSON (`chat-history.jsonl`, a separate file from `settings.json`). Append-only writes, a `static Mutex` serializes all read/append/clear, and corrupt JSON lines are skipped. `session_start` creates a conversation and `session_resume` appends to a stable conversation id; `read_conversations` returns least-recently-active → most-recently-active.
- `window.rs` centralizes main-window show/hide/focus behavior. The app keeps the window hidden instead of destroying it for instant recall.
- `translate/mod.rs` dispatches by `Engine` (`openai`/`google`/`bing`), pools clients by timeout/cookie mode, disables redirects, validates language codes and IPC payload sizes, and caps upstream response bodies. It also defines `ChatMessage` and `validate_messages` for both `chat` and `append_chat_history`.
- `translate/google.rs` calls the free `translate.googleapis.com` endpoint and parses its nested array response.
- `translate/bing.rs` calls the free `bing.com/ttranslatev3` endpoint. Bing needs a one-shot token (IG / IID / key / token) scraped from the translator page's HTML before POSTing, so it uses the cookie-store client and a browser User-Agent. Unofficial endpoint — may break or rate-limit.
- `translate/openai.rs` streams OpenAI-compatible SSE responses and emits `translate-chunk` (translation) or `chat-chunk` (AI chat) events to the frontend.
- `selection/macos.rs` captures the currently selected text via the macOS Accessibility API; falls back to a temporary `Cmd+C` + clipboard read if Accessibility fails. Non-macOS returns `None`.
- `screenshot.rs` invokes `/usr/sbin/screencapture -i -x -r` for Apple's native interactive free selection, writes to a random private temporary PNG, and treats Esc/no output as cancellation. `commands::screenshot_ocr` passes the PNG to Vision and removes it automatically afterward. Do not replace this architecture merely to address a permission prompt.
- `ocr/vision.rs` performs macOS-native OCR with Apple Vision (`VNRecognizeTextRequest`).
- `tray.rs` creates the menu bar tray menu (`翻译`, `设置`, `退出`).

### Svelte frontend

`src/App.svelte` holds two pieces of main-window view state: `view` (`main` / `settings`) and, within `main`, `mode` (`translate` / `chat`). On mount it loads chat history + appearance config, reads the current version, kicks off update checks, and subscribes to the five backend trigger events. Screenshot selection belongs to the native macOS service; there is no screenshot-selection frontend window.

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

- Screenshot OCR requires macOS Screen Recording permission. In dev mode the permission prompt may refer to the terminal or dev binary, so development success does not validate a packaged release identity. Selection capture separately prefers Accessibility and falls back to a temporary copy.
- OpenAI translation results stream via the `translate-chunk` event; AI chat streams via `chat-chunk`; Google and Bing return a complete string from the `translate` command.
- When the window is pinned (`set_pinned`), it stays always-on-top and does not auto-hide on blur.
- The Google and Bing endpoints are unofficial free endpoints and may be rate-limited or break (Bing scrapes a token from HTML, which is especially fragile).
- API keys live in the macOS Keychain, not the plaintext store. The store holds only non-sensitive config; the only trace of keys is transiently during the one-time plaintext→Keychain migration.

## Current Caveats

- Check `config.rs`, `shortcut.rs`, and `Settings.svelte` when behavior and prose diverge; security and release invariants in this file and `AGENTS.md` must stay aligned with the code.
- Rust unit tests cover: translation response/token parsing (Google, Bing), SSE parsing (OpenAI), selection text normalization, message validation, chat-history JSONL parsing + summary truncation + conversation splitting, and base_url normalization. Frontend unit tests cover the shortcut recorder (`shortcutRecorder.test.ts`), chat-context trimming (`chatContext.test.ts`), and the language list / adaptive target (`languages.test.ts`); no component-level test suite yet.
