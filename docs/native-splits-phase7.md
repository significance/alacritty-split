# Native splits: Phase 7 scrolling and yank keys

Bare PageUp/PageDown now scroll the focused pane on the primary screen, including in
vi mode. The new rows exclude both alternate screen and search. Existing Shift rows
are unchanged. Alternate-screen applications retain the bare keys; primary-screen
shell history bindings on these keys are displaced. Bind them to ReceiveChar to
restore upstream behavior.

The new vi-only Yank action copies a nonempty selection to the clipboard, clears the
selection and leaves vi mode through the existing context method (including IME and
redraw updates). Missing or empty selections do nothing. Default y uses Yank; Shift+Y
retains its selection/motion sequence and finishes with Yank. Search exclusions remain.
Action::Copy itself is unchanged; binding Copy plus ClearSelection restores the previous
stay-in-vi behavior.

## Validation

Linux aarch64, nightly-2026-04-01:

- cargo fmt --check: pass.
- cargo clippy --all-targets: pass.
- cargo test: 336 passed (150 app + 1 config + 7 derive + 132 terminal + 45 reference + 1 doctest).
- git diff --check: pass.

Seven new tests cover PageUp/PageDown modes and modifiers (including alternate screen
and search exclusions), exact vi y/Y action sequences, Yank TOML parsing, missing and
empty selections, mouse-style selection copying/clearing/mode exit, and the default Y
sequence from mid-line through the real terminal motion logic. The mock uses a no-op
clipboard backend and records the text passed to copy_selection.

No new macOS/Windows-gated code. Shared bindings and action execution compile on Linux;
native builds and clipboard/input behavior remain to be verified below. The unchanged
native IME/window behavior reached through toggle_vi_mode is not compiled on Linux.
No GUI smoke or native clipboard verification is claimed for this phase.

## Mac checklist (pending review)

- Build and run fmt, Clippy and all 336 tests; distinguish any known upstream lint.
- In a split with scrollback, bare PageUp/PageDown scroll only the focused pane;
  Shift variants still work. Repeat while in vi mode and while pane zoom is active.
- Run vim/less in an alternate screen: bare PageUp/PageDown reach the application.
- Open search: bare PageUp/PageDown retain upstream behavior; search navigation works.
- Enter vi mode, v + motion + y: expected text reaches the system clipboard, selection
  clears and typing goes to the shell. Repeat with a mouse selection in vi mode.
- y with no selection stays in vi mode and leaves existing clipboard content untouched.
- Shift+Y from mid-line copies through line end and leaves vi mode.
- Rebind PageUp/PageDown to ReceiveChar and y to Copy + ClearSelection; verify the
  documented upstream behavior can be restored.

Separate commit after Phase 6. Stop for Claude review.
