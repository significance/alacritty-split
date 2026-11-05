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

## Mac checks pending

- Check unchanged single-pane text placement, padding, search and message bars.
- Resize below the split minimum and change font size/DPI; shells must see current sizes.
- Check pane text clipping and one presentation per frame after rendering integration.
- Check Cmd+D and Cmd+Shift+D once actions are connected.
