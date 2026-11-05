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

## 2.3 Actions, lifecycle and focus

The actions SplitVertical, SplitHorizontal, ClosePane and FocusPaneLeft/Right/Up/Down
are bindable on every platform. macOS defaults are Cmd+D (new pane right), Cmd+Shift+D
(new pane below), Cmd+W (close focused pane), and Cmd+Option+arrows (directional focus).
The macOS pane binding table is constructed explicitly in tests.

A candidate tree validates the split before spawning a shell. New PTYs start at their
final pane size and inherit foreground-process CWD when available, falling back to the
configured directory. Spawn failure leaves the original tree unchanged. Exiting panes
collapse their tree parent and resize all survivors. Invalid geometry cannot strand a
dead pane: removal has a geometry-independent recovery, with a final rebuild preserving
all surviving terminal identities. Returning to one pane clears the cached native minimum.
Closing the final pane preserves the old explicit-quit/hold behavior.

Input processing keeps each pane's contiguous event batch under one terminal lock,
stopping at a pane command. Unprocessed events return to the queue in order. Window
commands run after locks drop, so subsequent keyboard events target the resulting focus.

Focus transitions update mode 1004 notifications, terminal focus flags, title, pointer
state and cursor blinking. Background panes retain their own title and visual bell;
clipboard access and hint updates are restricted to the focused pane. Native window
focus changes continue to target only that pane.

Mouse click-to-focus, divider dragging and pane-relative IME placement remain Phase 3.

## Validation

Formatting and Clippy pass on nightly-2026-04-01. The workspace has 315 passing tests
(129 app, 1 config, 7 derive, 132 terminal, 45 reference, 1 doctest).
Linux X11-only and Wayland-only builds are checked. A real X11/Xvfb smoke test
with software GL passed: three independent shells, vertical then nested horizontal
split, directional keyboard focus, input routing by shell PID, CWD inheritance,
explicit pane close, shell exit, survivor resizing and final window close (exit 0).
Initial new-shell grids were 32x50 and 16x50; the last survivor returned to 32x100.
Artifacts: /tmp/alacritty-split-smoke-j2mww13x; harness: /tmp/split-gui-smoke.py.
This exercised custom Linux bindings for the same actions, not native Mac keystrokes.
No pixel/visual inspection is claimed.

Claude independently verified the earlier 1c901b1e checkpoint on macOS: release
build and 310 tests pass, window dimensions and reference outputs match upstream,
and hold/IPC behavior passes. The new action stage has not yet been compiled on
macOS/Windows. Claude found two conditional unused arguments in swap_buffers; these
are now explicitly consumed under the macOS/Windows cfg. Their reported pre-existing
macOS collapsible_match lint remains untouched. New macOS-only lines append the tested pane binding table to the platform defaults
and replace the prior Cmd+W quit binding. The shared pane-command code and Unix CWD
lookup are compiled on Linux; the macOS caller and Windows fallback are uncompiled.
For the earlier sizing/rendering changes, no new platform-specific lines: the shared minimum-size
wrapper, renderer changes and existing macOS viewport/Windows swap paths remain
uncompiled on those platforms. The existing Wayland conditional also checks whether
full damage is required.

## Mac checks pending

- Run cargo clippy --all-targets on macOS (including the conditional swap-argument fix).
- Check unchanged single-pane text placement, padding, search and message bars.
- Check a single pane can shrink below its grid minimum as before.
- Resize below the split minimum and change font size/DPI; check shell resize behavior.
- Check nested pane text, selection, visual bell and search clipping.
- Check one presentation per frame, focused border and divider alignment at Retina scale.
- Check window messages render once across the full width, including over splits.
- Cmd+D: independent shell opens on the right; Cmd+Shift+D: independent shell below.
- Nest splits, then use Cmd+Option+arrows to move focus and verify the focused border.
- Cmd+W closes only the focused pane; shell exit collapses its split and resizes survivors.
- Check last-pane close and shell exit with and without --hold.
- Check CWD inheritance, failed split in a tiny window and failed PTY creation.
- Check title, OSC 52 clipboard isolation, focus reporting and blink behavior across panes.
