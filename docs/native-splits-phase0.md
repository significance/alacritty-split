# Native split panes: Phase 0 review

This personal fork now separates terminal sessions from native-window resources.
Phase 0 still opens exactly one pane. Split actions and the requested Cmd+D /
Cmd+Shift+D shortcuts are **not implemented yet**; they belong to Phase 2.

## Changes

| Step | Commit | Result |
| --- | --- | --- |
| 0.1 | `ad8325ce` | `Pane` owns terminal, PTY notifier, process identifiers, search and blink state; checked window-local ID allocation; PTY shutdown on pane drop. |
| 0.2 | `819812ea` | Events and terminal timers carry pane IDs; stale events are ignored; pane exit preserves other sessions and last-pane `--hold`; timer cleanup is scoped to pane/window. |
| 0.3 | `d2ef7884` | Each pane owns its `PaneView`; native window, GL surface/context, renderer, glyph cache and frame scheduling remain shared. |
| 0.4 | `e796aa91` | Explicit pane origins, pane-local projections and scissor bounds; separate window/pane clears; window message bar drawn outside pane clipping. |
| 0.5 | `84c3dc17` | Pure hit testing and pointer/touch translation; focused-pane input and outside selection drags preserved. |
| 0.6 | `6f4de2bd` | Config/IPC reload applies to every pane; shared font/DPI/window updates are consumed once and applied to each terminal; reference-test output iterates panes. |

The single-pane reference-test paths remain `grid.json`, `size.json` and
`config.json`. A future window with multiple panes writes each session under
`pane-<id>/` to avoid collisions. Config-reload cursor events now explicitly carry
window and pane IDs within the existing local event queue.

The input terminal remains locked across event processing, resizing and hint
refresh, as upstream did. Shared updates lock background terminals separately.
Renderer resize is still queued when `SizeInfo` changes, including physical pixel
extent; PTY/grid resize retains the existing rows/columns condition.

## Verification

Environment: headless Linux aarch64. Final toolchain:
`nightly-2026-04-01`, rustc `1.96.0-nightly (48cc71ee8 2026-03-31)`.
The latest nightly initially reported pre-existing Clippy failures in unchanged
`alacritty_terminal/src/tty/unix.rs`; the intended 1.96-era toolchain passes.
No unrelated terminal-library edits were made.

Untouched upstream `aecf0f60`: debug/release builds and all 268 tests passed.
The baseline test run used a separate clean `git archive` snapshot.

After every step, the following checks passed:

```sh
cargo +nightly-2026-04-01 fmt --check
cargo +nightly-2026-04-01 clippy --all-targets
cargo +nightly-2026-04-01 test
```

| Step | Total passing tests, including doctest |
| --- | ---: |
| 0.1 | 268 |
| 0.2 | 272 |
| 0.3 | 272 |
| 0.4 | 277 |
| 0.5 | 281 |
| 0.6 | 284 |

Added tests cover event origins across focus changes, timer isolation and cleanup,
GL Y conversion, full-window padding/scissor identity, offset pane viewports,
pointer boundaries/divider gaps, fractional/outside coordinates, shared font
metrics with different pane rectangles, pane-local search space, and tiny grids.

Final checks also passed:

```sh
cargo +nightly-2026-04-01 build --release
cargo +nightly-2026-04-01 check --no-default-features --features x11
cargo +nightly-2026-04-01 check --no-default-features --features wayland
```

No native GUI/rendering tests have been performed, and passing unit tests do not
establish pixel equivalence.

## Uncompiled platform paths

- macOS: initial/additional window and tab constructor plumbing, the
  `ActiveEventLoop` input-context argument, the existing viewport-reset call with
  explicit pane origin, shadow updates, and Option-as-Alt updates.
- Windows: window-before-GL construction and the omission of Unix PTY file
  descriptors/process IDs. Existing Windows terminal-library code is unchanged.
- No macOS default bindings were changed in Phase 0.

## Manual Mac checklist (all pending)

- [ ] Build on macOS and compare one-pane output against upstream with identical config/font/padding.
- [ ] Check text, cursor, colors, transparency, visual bell, and window resizing.
- [ ] Check search, vi mode, keyboard hints, URL hover/click, and the message bar while searching.
- [ ] Check selection dragging outside the window, wheel scrolling, copy/paste, and IME composition/cursor placement.
- [ ] Change font size and move the window between monitors with different DPI.
- [ ] Reload config, including font, padding, colors, cursor thickness and hint alphabet; check IPC config overrides/reset.
- [ ] Open multiple windows and macOS tabs; verify rendering and input remain isolated.
- [ ] Verify normal shell exit, Cmd+W, and `--hold` behavior, including the last window.
- [ ] Run `--ref-test` and confirm the existing three JSON output files remain compatible.

Build/run from the checkout without replacing the installed app:

```sh
cargo run --release
# Or, on macOS:
make app
open target/release/osx/Alacritty.app
```

## Next phases

Phase 1 adds and tests the pure binary layout tree. Phase 2 adds native split
actions, requested shortcuts, PTY creation, per-pane rendering and focus. Before
rendering multiple panes, presentation must be split into one window begin-frame,
N pane draws, and one window swap/frame request. Multi-pane Wayland rendering will
use full-window damage initially. Click focus/divider dragging and pane-relative
IME placement follow in Phase 3; divider config and user docs follow in Phase 4.

Phase 0 intentionally stops for review before any of those phases begin.
