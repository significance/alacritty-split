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

Click another pane to focus it. That first click is consumed, including with terminal
mouse reporting enabled. Later clicks use normal selection/reporting. Drag dividers to
resize panes. Wheel input scrolls the hovered pane without changing keyboard focus.

New shells inherit the foreground process's working directory when available. A split
that will not fit is rejected before starting another shell. Exiting a shell collapses
its split and resizes its neighbors.

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
# focused_edge_color = "#88c0d0"
```

Thicknesses are integer logical pixels. Zero hides the corresponding border; a zero-width
divider cannot be dragged. Omitted divider colors blend foreground/background 1:3;
omitted focused-edge colors follow the primary foreground. Settings reload live.
See [the example](../extra/panes.toml) and `alacritty(5)` for details.

`window.resize_increments` keeps its normal behavior for one pane. Cell snapping is
disabled while multiple panes exist, since those panes have different grids, and restored
when the window returns to one pane. Multi-pane pointer events currently acquire one
terminal lock per event; ordinary single-pane event batching is preserved.

## Verification

Linux tests and virtual-display functional checks pass. Claude has independently built
and tested the keyboard and mouse phases on macOS. Final native Mac visual/input checks
for current changes are tracked in [the Phase 4 report](native-splits-phase4.md).
