# Native split panes

This personal fork adds independent terminal panes within each Alacritty window.
Splits can be nested; each pane has its own shell, working directory, scrollback,
selection and search state.

| macOS shortcut | Action |
| --- | --- |
| Cmd+D | Split vertically: new pane on the right |
| Cmd+Shift+D | Split horizontally: new pane below |
| Cmd+Option+arrow | Focus the neighboring pane |
| Cmd+W | Close the focused pane; the last pane closes its window |
| Cmd+Shift+Enter | Zoom the focused pane; press again to restore tiling |
| Cmd+Enter | Toggle instant fullscreen without macOS Spaces |

Zoom fills the layout with the focused pane while preserving the tiling and split ratios.
Hidden shells keep running at their existing sizes until tiling is restored. The zoomed
pane retains its padding and focused outline, with no dividers. Split, close and
directional focus actions leave zoom first. A single-pane zoom shortcut does nothing.
Cmd+Control+F continues to toggle native macOS fullscreen.

Click another pane to focus it. That first click is consumed, including with terminal
mouse reporting enabled. Later clicks use normal selection/reporting. Drag dividers to
resize panes. Wheel input scrolls the hovered pane without changing keyboard focus.

New shells inherit the foreground process's working directory when available. A split
that will not fit is rejected before starting another shell. Exiting a shell collapses
its split and resizes its neighbors.

## Keys changed from upstream

- Bare PageUp/PageDown scroll the focused pane's primary screen, including in vi mode.
  Alternate-screen programs still receive them, and search behavior is unchanged.
  Shell history-navigation bindings on these keys are displaced. Bind PageUp/PageDown
  to `ReceiveChar` to restore upstream behavior. Shift+PageUp/PageDown still scroll.
- In vi mode, `y` copies the selection, clears it and returns to normal terminal mode.
  With no selection it stays in vi mode. `Shift+Y` selects to line end and yanks.
  Bind `y` back to `Copy` plus `ClearSelection` to keep vi mode after copying.

## Build and run

From the repository:

```sh
cargo build --release
cargo run --release
```

From the parent workspace, the justfile provides `just build` and `just run`; pass app
options with `just run --config-file path/to/alacritty.toml`. These commands run this
checkout without replacing an installed Alacritty. On macOS, `make app` builds the
separate `target/release/osx/Alacritty.app` bundle.

## Appearance

Each split pane gets one logical pixel of extra padding on every side, scaled for
Retina/high-DPI displays. This adds to `window.padding` and disappears when only one pane
remains. Configure it in the regular Alacritty TOML file:

```toml
[panes]
padding = 1
divider_thickness = 1
focused_edge_thickness = 1
# divider_color = "#404040"
# focused_edge_color = "#4b2070"
```

Thicknesses are integer logical pixels. Zero hides the corresponding border; a zero-width
divider cannot be dragged. Omitted divider colors blend foreground/background 1:3;
omitted focused-edge colors use dark purple (`#4b2070`). Settings reload live.
See [the example](../extra/panes.toml) and `alacritty(5)` for details.

A theme can give the active pane a different outline:

```toml
[panes]
focused_edge_color = "#88c0d0"
```

Include this table in your theme file alongside its `[colors]` tables. The active pane
gets this outline; inactive panes retain the muted divider colour. The dark-purple
default reads on both dark and light backgrounds.

`window.resize_increments` keeps its normal behavior for one pane. Cell snapping is
disabled while multiple panes exist, since those panes have different grids, and restored
when the window returns to one pane. Multi-pane pointer events currently acquire one
terminal lock per event; ordinary single-pane event batching is preserved.

## Verification

Linux tests and virtual-display functional checks pass. Claude has independently built
and tested the keyboard and mouse phases on macOS. Final native Mac visual/input checks
for pane zoom and key changes are tracked in [Phase 6](native-splits-phase6.md) and
[Phase 7](native-splits-phase7.md).
