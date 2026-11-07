# Native splits: Phase 8 separate macOS windows

Initial windows, Cmd+N and IPC create-window requests without a tabbing identifier
now set NSWindowTabbingMode::Disallowed before the window is first shown. Immediately
after set_visible(true), they restore Automatic so later Cmd+T tabs can join them.
Explicit tab requests retain winit's Preferred mode throughout. The window records
whether its original request included a tabbing identifier before that option is taken.
No configuration key or Linux/Windows behavior changes.

The existing macOS binding table is now available to unit tests on every platform;
production defaults are unchanged. A new test pins Cmd+N -> CreateNewWindow and
Cmd+T -> CreateNewTab, with exact modifiers and no mode restrictions.

## Validation

Linux aarch64, nightly-2026-04-01:

- cargo fmt --check: pass.
- cargo clippy --all-targets: pass.
- cargo test: 337 passed (151 app + 1 config + 7 derive + 132 terminal + 45 reference + 1 doctest).
- git diff --check: pass.

Logs: /tmp/splits-newwindow-{clippy,test}.log. Fetched the locked macOS dependencies
and inspected objc2-app-kit 0.3.x: NSWindowTabbingMode::{Disallowed, Automatic} and the
safe setTabbingMode method exist with the used signature. This is source inspection,
not a macOS compilation or AppKit behavior test.

## Uncompiled paths

All new window-placement behavior is cfg(target_os = "macos") and uncompiled on Linux:

- NSWindowTabbingMode import, Window::tabbing_requested field and initialization.
- Capturing the tab request and setting Disallowed in Window::new.
- Window::allow_tabbing and the raw-handle -> NSView -> NSWindow helper.
- Display::new's post-show allow_tabbing call.

The macOS production binding wrapper is also uncompiled here; its shared table and
new binding test compile and run on Linux. No new Windows-gated lines. Native Mac
build and AppKit behavior verification are required before accepting the fix.

## Mac checklist (pending review)

- Build release; run fmt, Clippy and 337 tests, separating any known upstream lint.
- Set Desktop & Dock > Prefer tabs when opening documents to Always: launch and
  Cmd+N create separate windows, while Cmd+T creates a tab in the current window.
- Repeat Cmd+N/Cmd+T in Cmd+Enter simple fullscreen and native fullscreen.
- With the preference In Full Screen, repeat both fullscreen modes.
- alacritty msg create-window creates a separate window without changing the old
  window's pane layout, focus state or zoom state.
- After Cmd+N, Cmd+T on the new window creates a tab in that window. Repeat from an
  existing tab; native tab switching and tab/window close still work.
- Restore the user's system preference after verification.

Stop for Claude's native Mac review.
