# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

ttree is a lightweight macOS AI translator built with **Tauri 2 + Svelte 5 + Rust**. It is designed as a hidden spotlight-style app: the main window starts hidden, global shortcuts show it instantly, and the window auto-hides on blur.

Core features:
- `CmdOrCtrl+Shift+Space` by default toggles the translation window.
- `CmdOrCtrl+Shift+S` by default triggers screenshot OCR translation.
- Translation engines: Google free web endpoint and OpenAI-compatible streaming chat completions.
- macOS OCR uses Apple Vision via `objc2-vision` (`VNRecognizeTextRequest`).
- Settings persist locally through `tauri-plugin-store`; shortcuts are user-configurable and re-registered immediately after saving.

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

Build outputs are under `src-tauri/target/release/bundle/`, including `bundle/macos/ttree.app` and `bundle/dmg/*.dmg`.

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
4. CI (`.github/workflows/release.yml`) builds `aarch64-apple-darwin` + `x86_64-apple-darwin`, signs with `TAURI_SIGNING_PRIVATE_KEY` (repo secret), and creates the GitHub Release with `latest.json`.
5. Watch the run: `gh run watch -R ZASENJC/ttree` (or check https://github.com/ZASENJC/ttree/actions). Confirm the Release exists and lists `latest.json` + the signed `.tar.gz`/`.sig` assets.
6. Installed apps auto-check for updates on startup and every 6h, hitting the network only once ≥7 days since the last check; users install from 设置 → 通用. New installs get the latest Release immediately.

**Prerequisites (already done, do not repeat):** signing keys live at `~/.config/ttree-updater/` on this machine; the public key is in `tauri.conf.json` and the private key is in the `TAURI_SIGNING_PRIVATE_KEY` repo secret. The private key **must carry a strong passphrase**: without a second factor, anyone who exfiltrates the key can sign an updater package that installed apps auto-accept (RCE). The passphrase lives in macOS Keychain (account `updater-signing-password`, service `com.samwstu.ttree`) for local builds and in the `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` repo secret for CI. To build a signed update locally instead of via CI: `source ./scripts/signing-env.sh && npm run tauri build` — the script reads the passphrase from Keychain automatically. If the key was ever stored unencrypted, rotate it: generate a new minisign keypair with a passphrase, update `tauri.conf.json` pubkey, and ship one release on the old key that bumps users to the new key.

## Architecture

### Tauri / Rust backend

`src-tauri/src/lib.rs` is the backend composition point. It registers plugins, commands, global shortcuts, tray setup, macOS vibrancy, and the blur-to-hide window behavior.

Important backend modules:

- `commands.rs` exposes Tauri commands used by the Svelte frontend:
  - translation (Google / OpenAI streaming)
  - AI chat (`chat`, `get/set_chat_ai_config`) — separate config from translation
  - screenshot OCR
  - show main window
  - get/set OpenAI translation config
  - get/set shortcut config, `set_shortcut_recording` (suppresses global shortcuts during key capture)
  - `get_pinned` / `set_pinned` — window always-on-top + blur-no-hide toggle
- `shortcut.rs` builds the global shortcut plugin and dispatches five registered shortcuts: `toggle` (show/hide window, emits `trigger-translate` on show to reset the frontend to the translate view), `ocr` (screenshot OCR), `ai_dialog` (open AI chat), `selection_translate` (capture selected text and translate), and `selection_ai_dialog` (capture selected text and send to AI). All are optional. Two pairs may intentionally share the same key — `toggle`↔`selection_translate` and `ai_dialog`↔`selection_ai_dialog`; on collision the selection action wins (dispatch order in `ParsedShortcuts::entries` lists selection actions first), so a shared key runs the selection behavior when text is selected and falls back to the plain toggle/dialog otherwise. All other key collisions are rejected at validation. Startup uses persisted shortcut config; saving validates, unregisters existing shortcuts, and registers the deduplicated set immediately.
- `config.rs` owns persisted config models (`OpenAiConfig`, `ChatAiConfig`, `ShortcutConfig`) and store reads/writes through `tauri-plugin-store`. Translation and chat use separate AI config entries; each carries its own optional prompt (`translate_prompt`, `chat_prompt`). `ShortcutConfig` holds five independently-optional shortcuts: `toggle`, `ocr`, `ai_dialog`, `selection_translate`, `selection_ai_dialog` (empty string disables one).
- `window.rs` centralizes main-window show/hide/focus behavior. The app keeps the window hidden instead of destroying it for instant recall.
- `translate/google.rs` calls the free `translate.googleapis.com` endpoint and parses its nested array response.
- `translate/openai.rs` streams OpenAI-compatible SSE responses and emits `translate-chunk` (translation) or `chat-chunk` (AI chat) events to the frontend.
- `selection/macos.rs` captures the currently selected text via the macOS Accessibility API; falls back to a temporary `Cmd+C` + clipboard read if Accessibility fails. Non-macOS returns `None`.
- `screenshot.rs` shells out to `/usr/sbin/screencapture -i -x -r` for interactive region capture.
- `ocr/vision.rs` performs macOS-native OCR with Apple Vision (`VNRecognizeTextRequest`).
- `tray.rs` creates the menu bar tray menu (`翻译`, `设置`, `退出`).

### Svelte frontend

`src/App.svelte` switches between the translation panel and settings view. It also listens for tray `open-settings` events.

Important frontend areas:

- `src/lib/components/TranslatePanel.svelte` is the main spotlight UI. It handles Enter-to-translate, Esc-to-hide, engine/language selectors, and listens for screenshot OCR shortcut events.
- `src/lib/views/ChatPanel.svelte` is the AI chat view. It sends multi-turn messages and streams the assistant reply via `chat-chunk` events.
- `src/lib/stores/translation.svelte.ts` and `src/lib/stores/chat.svelte.ts` own their respective feature state. Both use the `.svelte.ts` extension because they contain Svelte 5 runes (`$state`). Do not move rune-based stores back to plain `.ts`; doing so leaves `$state` uncompiled and causes a runtime white screen.
- `src/lib/shortcuts/shortcutRecorder.ts` is a pure utility that converts keyboard events into Tauri-style shortcut strings. It has a Vitest unit test (`shortcutRecorder.test.ts`).
- `src/lib/api/tauri.ts` is the frontend boundary for all `invoke` calls and event subscriptions.
- `src/lib/views/Settings.svelte` edits OpenAI config, custom shortcuts, and autostart.
- Styling is based on CSS custom properties in `src/styles/tokens.css` plus global rules in `src/styles/global.css`.

## Runtime Notes

- Screenshot OCR requires macOS Screen Recording permission. In dev mode the permission prompt may refer to the terminal or the dev binary.
- OpenAI translation results stream via the `translate-chunk` event; AI chat streams via `chat-chunk`; Google returns a complete string from the `translate` command.
- When the window is pinned (`set_pinned`), it stays always-on-top and does not auto-hide on blur.
- The Google endpoint is an unofficial free endpoint and may be rate-limited.
- API keys are stored in the local Tauri store; there is not yet a Keychain integration.

## Current Caveats

- The README may lag behind implementation details; check `config.rs`, `shortcut.rs`, and `Settings.svelte` for the current shortcut behavior.
- There are Rust unit tests for translation response parsing, SSE parsing, and selection text normalization. Frontend unit tests cover the shortcut recorder utility (`src/lib/shortcuts/shortcutRecorder.test.ts`); no component-level test suite yet.
