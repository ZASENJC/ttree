# Shortcut Recorder Implementation Plan

**Goal:** Let users record shortcut combinations by pressing keys instead of typing shortcut syntax.

**Architecture:** Keep canonical shortcut strings in a shared recorder model, with readable platform labels for display. Native Windows and macOS controls capture keyboard events, preserve Tab navigation and cancel/clear commands, and suspend this application's global registrations while recording. Existing save, conflict validation, defaults and settings format remain in use.

**Tech Stack:** Rust, global-hotkey, Win32 edit subclass, AppKit NSTextField subclass.

## Implementation

1. Add `crates/ttree-platform/src/shortcut_recorder.rs` with recorder state, display formatting and capture tests for modifier-only input, combinations, clear, cancel and navigation.
2. Add a Windows recorder adapter and connect the five shortcut fields to canonical values. Block typed/pasted text, add focus cues, and suspend/resume app hotkeys on focus changes.
3. Add an AppKit shortcut field and connect the same interactions and suspension behavior. Preserve platform-specific input mapping.
4. Run meaningful recorder tests, native Windows compilation and interaction checks, then build a portable Windows artifact. Report macOS verification limits accurately.
5. Stage only this task's changes, preserve parallel window-mode edits, and commit after validation.

## Verification

- The exported staged snapshot passes all 59 workspace tests.
- Both debug and release Windows builds pass 26 isolated keyboard interaction checks, including save, duplicate rejection, suspension and restoration of existing bindings, defaults, and preserving the user configuration.
- The AppKit recorder module passes cross-target type checking for aarch64-apple-darwin. Full macOS application checking stops in native C dependencies on this Windows host; Mac runtime interaction remains unverified.
- Windows x64 GUI executable and portable ZIP contents are verified against the release build.
