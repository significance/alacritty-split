# Native splits: Phase 2 integration

## 2.1 State and sizing

WindowContext owns the layout tree and cached geometry. Shared font metrics,
configured padding and window-level message height produce LayoutParams. Geometry is
computed once per update, then used for pane resizing and rendering.

Single-pane windows retain the upstream ability to shrink below the terminal grid
minimum; they have no native minimum size. Multiple panes request the recursive layout
minimum plus message-bar height. Display caches this hint and updates winit only when
it changes. Too-small compositor resizes use minimum-sized virtual bounds, clipped to
the actual native surface. PTYs see the virtual pane grid dimensions when clipped,
not the smaller visible region. Renderer surface resizing is explicitly requested only in
this clipped case; normal resizing still follows the SizeInfo comparison.

PTY resize notifications retain the upstream grid-change condition. The earlier
pixel-metric notification change in 6ef2caf9 was reverted after Review 12 to avoid
additional SIGWINCH notifications during pixel-by-pixel window dragging.

The module-wide dead-code suppression was replaced by annotations on the specific
layout actions and helpers awaiting the next integration checkpoint. Layout minimums
are at least one physical pixel, including with zero metrics. Glyph cell heights are
currently whole physical pixels; subtracting message lines preserves dynamic padding.

## 2.2 Rendering

Rendering has one begin-frame, a draw call per pane, window-level divider and focus
borders, then one presentation. Message rendering and frame scheduling occur once.
All panes advance their damage history after presentation. Wayland uses a full swap
for multiple panes or a visible message bar; the latter is an intentional conservative
change for single-pane windows because the message is now a window overlay and the
pane damage coordinates exclude its reserved area.

Dividers and focus edges scale with window DPI, with a minimum of one physical pixel.
Dividers use a blend of one part foreground to three parts background; focus edges use
foreground. Focus edges stay inside the pane and are hidden for a single pane.
The renderer consumes the pure decorations helper tested against an explicit three-pane
layout, including a focused pane touching both dividers and the window edge.

The render timer, hyperlink preview and meter sample run only for the focused pane.
The meter continues measuring grid drawing, now explicitly of the focused pane; it does
not represent total multi-pane frame time. Keeping the preview in that pane's draw call
preserves its clipping, cursor-relative placement and captured terminal state without
holding a terminal lock through presentation. No terminal is re-locked for these overlays.

## Validation

Formatting and Clippy pass on nightly-2026-04-01. The workspace has 310 passing tests
(124 app, 1 config, 7 derive, 132 terminal, 45 reference, 1 doctest).
Linux X11-only and Wayland-only builds are checked. No GUI validation or native
macOS/Windows compilation. No new platform-specific lines: the shared minimum-size
wrapper, renderer changes and existing macOS viewport/Windows swap paths remain
uncompiled on those platforms. The existing Wayland conditional also checks whether
full damage is required.

## Mac checks pending

- Check unchanged single-pane text placement, padding, search and message bars.
- Check a single pane can shrink below its grid minimum as before.
- Resize below the split minimum and change font size/DPI; check shell resize behavior.
- Check nested pane text, selection, visual bell and search clipping.
- Check one presentation per frame, focused border and divider alignment at Retina scale.
- Check window messages render once across the full width, including over splits.
- Check Cmd+D and Cmd+Shift+D once actions are connected.
