# Native splits: Phase 4 styling and final polish

The [panes] table now controls padding, divider thickness/colour and focused-edge
thickness/colour. Dimensions are integer logical pixels, DPI-scaled. Border defaults
match Phase 3: one logical pixel, a 1:3 foreground/background divider blend, and primary
foreground for the focused edge. The user explicitly requested about 2px inside every
pane, so padding intentionally defaults to two logical pixels per side while split,
added to window.padding. Returning to a single pane restores its previous padding.
Geometry minima, new PTY initial sizes and resized panes use these same insets.
Config reload and IPC apply the new settings to existing panes.

Review 16/17 fixes:

- Window messages are handled only in window coordinates while split. The pane-local
  upstream message hit test is disabled, preventing false hits in upper-pane padding.
- Wheels target the hovered pane without changing keyboard focus. A separate per-pane
  wheel Mouse preserves fractional wheel accumulation without disturbing focused input.
- Bell urgency uses native-window focus instead of per-pane focus. Background-pane
  visual bells still render in their own pane.
- Rejected splits no longer consume an identity. Focus-out to an exited pane's closed
  notifier is documented as harmless.
- Cell resize increments are cached, disabled while split and restored for one pane.
- Multi-pane pointer events intentionally take one terminal lock per event. Ordinary
  single-pane batching is preserved.

The guide, both relevant manpages, changelog, README fork note and example config are
updated. See native-splits.md and extra/panes.toml.

## Validation

Pinned nightly-2026-04-01 on Linux aarch64:

- cargo fmt --check: pass.
- cargo clippy --all-targets: pass.
- cargo test: 324 passed (138 app + 1 config + 7 derive + 132 terminal + 45 reference + 1 doctest).
- X11-only and Wayland-only cargo check: pass.
- Debug build: pass.

Four new pure tests cover default/configured pane styling, DPI scaling and zero-thickness
edges; split padding plus restoration to the single-pane size; and the pane-local false
message-bar hit. Existing layout and pointer tests continue passing.

Live X11/Xvfb/software-GL smoke passed with mode 1003 enabled:

- Vertical and nested horizontal splits; normal and focus-only clicks; both drag axes.
- Wheel reports reached the hovered background shell, while subsequent keyboard text
  still reached the original focused shell.
- Native X11 WM_NORMAL_HINTS had resize increments with one pane, omitted them while
  split, and restored them after returning to one.
- IPC changed padding to 12, shrinking a shell from 31x49 to 30x47; reset restored 31x49.
- IPC create-window opened a second native window. Its shell exited and that window
  closed while the original kept running. Final original-window close exited cleanly.
- New split shells inherited /tmp; pane exit and resizing of survivors still passed.

Harness: /tmp/split-final-smoke.py. Artifacts: /tmp/alacritty-split-smoke-j5mrbate.
No pixel inspection is claimed. macOS tab code was not modified and is not compiled on
Linux. There are no new macOS/Windows-gated lines in this phase; the shared optional
resize-increment setter, wheel routing, bell state and styling remain uncompiled on those
platforms at this checkpoint. Claude's previous Mac builds/tests accepted Phases 2/3.

## Final Mac checklist (pending)

- Build release and run fmt, tests and Clippy; distinguish the known upstream lint.
- Cmd+D/Cmd+Shift+D, nested splits, Cmd+Option+arrows, Cmd+W and final-pane --hold behavior.
- Verify the new 2px-per-side inset on standard and Retina displays; confirm the last
  pane returns to its original padding.
- Reload all panes keys (including zero borders and custom colours), plus IPC config/reset.
- Click-focus in vim mouse mode, hovered-pane wheel scrolling, both divider drag axes,
  selection across dividers, hints, URL clicks and clipboard isolation.
- Trigger a background-pane bell while the window is focused: no native urgency bounce.
- Show a window message, click upper-pane padding, then use the actual window close button.
- IME composition/candidate placement in right/bottom panes and across focus changes.
- Resize/font-size changes, minimum sizes, DPI changes and snapping restoration.
- Independent windows, IPC create-window and native macOS tabs.

Stop here for the Mac review and final visual/input checks before further changes.
