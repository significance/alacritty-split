# Native splits: Phase 2 integration

## 2.1 State and sizing

WindowContext now owns the layout tree. Shared font metrics, configured padding and
window-level message height produce LayoutParams; geometry drives each pane rectangle.
The native minimum size includes the recursive layout minimum and message bar. A
compositor resize below that minimum uses valid virtual bounds clipped by the native
surface, while every pane still receives current terminal and PTY dimensions.
PTY pixel metrics are updated even when the row/column count stays unchanged. Native
surface resizing is queued independently of pane geometry changes.

The module-wide dead-code suppression has been replaced by annotations on the specific
layout actions and helpers awaiting the next integration checkpoint.

Validation: 308 workspace tests pass, including message/grid/padding equivalence and
layout-input rounding/saturation tests. Formatting and Clippy run on pinned
nightly-2026-04-01. No GUI validation. No macOS/Windows-specific lines added; the shared
winit minimum-size wrapper has only been compiled on Linux.

## 2.2 Rendering

Window rendering now has one begin-frame, a draw call per pane, window-level divider
and focus borders, then one presentation. Message rendering and frame scheduling occur
once. All panes advance their damage history only after presentation. Wayland uses a
full swap for multiple panes or a visible message bar; pane-local damage is retained
for the ordinary single-pane path. The focused-pane border is one physical pixel inside
its bounds and is hidden in the single-pane case. The pure border helper is tested for
normal, tiny and empty rectangles; divider geometry is covered by the layout tests.

Validation: formatting and Clippy pass; 309 workspace tests pass (123 app, 1 config,
7 derive, 132 terminal, 45 reference, 1 doctest). GUI rendering is unverified. No new
platform-specific lines were added. The shared rendering refactor, including the
existing macOS viewport restoration path and Windows swap path, is compiled on Linux
only. The existing Wayland conditional now also checks whether full damage is required.

## Mac checks pending

- Check unchanged single-pane text placement, padding, search and message bars.
- Resize below the split minimum and change font size/DPI; shells must see current sizes.
- Check nested pane text, selection, visual bell and search clipping.
- Check one presentation per frame, focused border and divider alignment.
- Check window messages render once across the full width, including over splits.
- Check focus-border edges at Retina scale and window resize/font changes.
- Check Cmd+D and Cmd+Shift+D once actions are connected.
