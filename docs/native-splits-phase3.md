# Native splits: Phase 3 mouse and IME

Clicking an unfocused pane changes focus and consumes both edges of the click.
The next click uses the existing terminal mouse/selection behavior. Focus-click
coordinate seeding does not emit synthetic PTY motion, including mode 1003.
Selection drags keep their original pane across dividers; keyboard focus changes
cancel capture and consume outstanding releases. Selection-scroll timers are cancelled
when focus changes. Pointer events are routed in queue order against current geometry.
Ordinary single-pane events retain the existing contiguous-event batching.

Dividers use column/row resize cursors. A drag keeps its initial grab offset and
uses the layout's subtree minimums to clamp movement. Nested vertical and horizontal
dividers resize all affected terminals. Divider gestures never reach a terminal.
Mouse wheels over inactive panes/dividers do not go to the focused terminal; click to
focus that pane before scrolling it. Window messages use a window-coordinate hit test,
so their close button remains usable with any focused-pane origin.

IME cursor coordinates now include the pane origin, preserving X11's one-cell vertical
offset. Only the focused pane updates the native IME position. Native composition
state follows pane focus, and the focused pane's vi/search state updates the native
IME inhibitor. No macOS/Windows-only code was added in this phase, but these shared
paths have only been compiled on Linux for this phase.

## Validation

Pinned nightly-2026-04-01:

- cargo fmt --check: pass.
- cargo clippy --all-targets: pass.
- cargo test: 320 passed (134 app + 1 config + 7 derive + 132 terminal + 45 reference + 1 doctest).
- X11-only and Wayland-only cargo check: pass.
- Debug build: pass.

Five new pure tests cover focus-click press/release suppression, divider grab offset,
selection capture/cancelled releases, window message coordinates, and pane-relative IME
coordinates with the X11 offset.

Live X11/Xvfb/software-GL mouse smoke test: pass. Three shells enabled SGR mouse
reporting with mode 1003. Focus clicks delivered no mouse sequences, subsequent clicks
reported press/release normally, and both divider orientations resized the intended
shells. The left pane grew from 49 to 65 columns; a nested bottom pane shrank from 16 to
9 lines. Click-focus input went to the expected shell PIDs. Pane close, shell exit and
last-window close still passed. Harness: /tmp/split-mouse-smoke.py; artifacts:
/tmp/alacritty-split-smoke-y6nq0b0x. This verifies behavior, not pixel appearance or native
macOS IME integration.

## Mac checks still needed

- Build and run without replacing the installed app: cargo run --release.
- Run cargo clippy --all-targets; report any new platform-specific warnings.
- Cmd+D, Cmd+Shift+D, nested splits, Cmd+Option+arrows and Cmd+W.
- Click another pane in vim with mouse reporting enabled; first click only focuses.
- Drag both divider orientations, including nested splits and minimum-size limits.
- Drag selections across a divider; copy/paste and URL/hint clicks in the focused pane.
- Close a window message while a right/bottom pane is focused.
- Enter IME composition in right/bottom panes and after focus changes; check candidate placement.
- Check Retina/multi-monitor DPI, transparency, search, scrollback and multiple windows/tabs.

Configurable divider styling and resize-increment policy remain Phase 4.
