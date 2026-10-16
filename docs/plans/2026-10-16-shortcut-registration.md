# Shortcut Registration Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix Windows shortcut registration for `CmdOrCtrl+Right` and preserve working shortcuts when settings cannot be applied.

**Architecture:** Delegate shortcut aliases to global-hotkey's platform-aware parser. Share incremental registration between the native clients: keep existing bindings, register additions, release obsolete bindings only after success, and roll back a failed change. Preserve action dispatch and the existing settings format.

**Tech Stack:** Rust Cargo workspace, global-hotkey 0.8, Win32, AppKit.

---

### Task 1: Reproduce modifier parsing

- Add platform-sensitive regression tests in `crates/ttree-platform/src/hotkey.rs` for the screenshot's Right and Shift+S combinations, default shortcuts, aliases, explicit Cmd/Option, and equivalent-key conflicts.
- Run `cargo test -p ttree-platform --locked`; confirm Windows fails because CmdOrCtrl becomes Super.

### Task 2: Fix parsing and registration

- Remove the redundant alias conversion from `crates/ttree-platform/src/hotkey.rs`; parse the trimmed input with global-hotkey.
- Add a shared registration helper that retains unchanged keys, stages additions, reports the conflicting action/key, and restores the previous registration set after failure.
- Add native Windows regression coverage with a second manager holding a conflicting key. Verify rollback, retry, repeated save, swapping actions, shared bindings and disabling shortcuts.

### Task 3: Update callers

- Use the helper from `apps/windows/src/app.rs` and `apps/macos/src/state.rs` when applying settings.
- Surface startup registration errors and registration/persistence errors in settings, without changing user config locations or defaults.
- Inspect both callers; only native Windows execution is available on this host.

### Task 4: Verify and deliver

- Run platform tests, `cargo check --workspace --locked`, `cargo test --workspace --locked`, and formatting/diff checks appropriate to the changed files.
- Build a Windows executable when local prerequisites are available. Keep toolchains, logs and binaries under ignored `target/` and `dist/`.
- Report the root cause, changed files, verification results and any unverified platform behavior. Do not publish or push a release.

## Verification results

- Before the parser fix, three regression tests failed on Windows: platform aliases, default modifiers, and equivalent-modifier conflicts. The actual modifier was Super rather than Control.
- After the fix, `cargo check --workspace --locked` and `cargo test --workspace --locked` passed on native Windows (55 tests in the final run). The native registration regression covers external ownership, rollback, retry, repeated saves, action swaps, shared keys and disabling keys.
- `cargo build -p ttree-windows --release --locked` passed with a portable Rust/MSVC toolchain under ignored `target/tools`. Changed Rust files passed rustfmt checks; `git diff --check` passed.
- A native probe confirmed Windows rejects the old Super+Right and Super+Shift+S mappings. The corrected Control+Right mapping registered successfully, including a repeated save. Control+Shift+S was occupied while a TTREE instance was already running; that process was preserved.
- The portable ZIP at `dist/windows/TTREE-hotkey-fixed.zip` contains the built executable and license files. ZIP structure and launching the packaged executable with `--hidden` to reuse the existing instance passed.
- macOS callers use the same registration helper, but native macOS compilation and interaction were not available on this Windows host.
- Existing parallel changes to Windows startup, layout, autostart and documentation were preserved. No GitHub push or release was performed.
