# Native splits: Phase 6 pane zoom

Cmd+Shift+Enter toggles the focused pane between the full layout area and its original
tiling. The new TogglePaneZoom action is bindable on every platform. Cmd+Enter now uses
the existing ToggleSimpleFullscreen action on macOS (instant, without Spaces);
Cmd+Control+F still uses ToggleFullscreen.

Zoom is window-local presentation state. It never mutates the layout tree, ratios or
pane order. Visible geometry contains just the zoomed pane and no dividers. Hidden
panes retain their PTY sizes and keep processing output and events. Window, font,
padding and message-bar changes resize only the visible pane; leaving zoom resizes the
whole tiling. The zoomed pane retains split padding and the configurable focused edge.
Split, close and directional focus leave zoom first. A hidden shell exit preserves zoom;
a zoomed shell exit restores tiling. If the last hidden shell exits, the surviving zoom
retains its inset and indicator until toggled off. Starting zoom with one pane is a no-op.

## Validation

Linux aarch64, nightly-2026-04-01:

- cargo fmt --check, cargo clippy --all-targets: pass.
- cargo test: 329 passed (143 app + 1 config + 7 derive + 132 terminal + 45 reference + 1 doctest).
- X11-only and Wayland-only cargo check: pass.
- Debug build: pass.

Five new pure tests cover exact layout restoration (including non-half ratios), visible-only
size updates, split/close/focus transitions, hidden and visible exits, and single-pane
no-op. Existing binding-table, action parsing and input mapping tests include zoom.
The table explicitly checks the distinct modifiers for both Enter defaults.

Live X11/Xvfb/software-GL smoke passed using a custom Linux zoom binding:

- Single-pane toggle no-op; vertical and nested horizontal splits; zoom and exact grid restoration.
- Hidden PTY sizes unchanged through zoom, IPC font-size and padding changes, and reset.
- Directional focus leaves zoom and routes keyboard input to the neighbor.
- Split while zoomed restores tiles before spawning a fourth shell; close restores three.
- Hidden shell exit preserves zoom and the other hidden shell's size.
- Zoomed shell exit restores the survivor's original single-pane grid; final exit is clean.

Harness: /tmp/split-zoom-smoke.py. Artifacts: /tmp/alacritty-split-smoke-h9t6v363.
Logs: /tmp/splits-zoom-{test,clippy,build,x11,wayland}.log.
No pixel inspection or native fullscreen verification is claimed.

The new default rows are in a shared, unit-tested table, but their macOS-only insertion
into default bindings is not compiled here. The existing macOS ToggleSimpleFullscreen
execution path is unchanged and uncompiled here. There are no new Windows-gated lines.
Shared WindowContext changes still need native macOS/Windows compilation and GUI checks.
IPC create-window, ref-test and native tab creation paths are unchanged; previous phase
checks cover independent windows, but the native multi-window/tab regression remains below.

## Mac checklist (pending review)

- Build release; run fmt, Clippy and all 329 tests, separating any known upstream lint.
- Zoom/unzoom with two and four panes, including dragged ratios; verify exact restoration.
- Cmd+Enter toggles instant fullscreen without Spaces; Cmd+Shift+Enter only zooms panes;
  Cmd+Control+F retains native fullscreen behavior. Test zoom inside both fullscreen modes.
- Run continuous output in a hidden pane; unzoom and confirm output and scrollback survived.
- Cmd+D/Cmd+Shift+D, Cmd+W and directional focus while zoomed; verify restored tiling/focus.
- Exit hidden and zoomed shells separately, including the last hidden shell and --hold.
- Resize the window, change font size and monitor DPI while zoomed; hidden PTY grids stay
  fixed until unzoom and then catch up once.
- Reload/IPC-change panes styling while zoomed; verify padding, outline and absence of dividers.
- Show/dismiss the message bar while zoomed; verify full-width bar and correct layout bounds.
- Click, wheel, selection, terminal mouse reporting and IME placement in the zoomed pane;
  no divider cursor or drag should remain. Check focused outline and one-logical-pixel inset.
- Independent windows, IPC create-window, --ref-test and native macOS tabs remain unaffected.

Stop for Claude's review before further changes.
