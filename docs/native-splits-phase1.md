# Native split panes: Phase 1 review

Phase 1 adds the pure layout tree in `alacritty/src/pane/layout.rs`. It has no
window-system or OpenGL calls, new dependencies, actions, or shortcuts. Native
pane creation and Cmd+D / Cmd+Shift+D are still Phase 2.

## Behavior

- Binary leaf/split tree with exact rational weights and integer pixel bounds.
- Vertical splits put the new pane on the right; horizontal splits put it below.
- Rounding gives the first child the floor and the second the remainder.
- Caller-supplied `MinSize` includes whatever cell dimensions/padding are needed.
- Splits must fit within the target pane; neighboring panes cannot lose space.
- Split, close, and divider-drag changes are validated before committing. Failures
  preserve the exact tree, including its ratios.
- Resize calculation is immutable. It clamps actual child extents to subtree
  minimums without discarding stored ratios, or returns `TooSmall`.
- Closing collapses the parent and returns the nearest leaf in the sibling
  subtree. Removing the last leaf returns an explicit error.
- Directional focus prefers overlapping edges, then axis gap and center
  alignment, overlap length, center distance, and lowest pane ID.
- Divider paths identify tree nodes until the next split/close; recompute them
  after structural changes.

## Verification

On `nightly-2026-04-01` (Rust 1.96.0-nightly):

```sh
cargo +nightly-2026-04-01 fmt --check
cargo +nightly-2026-04-01 clippy --all-targets
cargo +nightly-2026-04-01 test
```

All pass: **306 total tests**, including **22 new layout tests**.
The property-style test performs 12,288 operations across 32 deterministic seeds,
reaches six-pane layouts, and checks bounds, minimums, pairwise non-overlap, exact
pixel coverage, rollback on failure, and valid focus targets. Leaf/divider areas
sum to the window area, proving full coverage when combined with the bounds and
non-overlap checks. Tests also cover odd pixels, offsets, zero-width dividers,
nested minimums, tiny/overflowing bounds, extreme drags, missing/duplicate panes,
last-pane removal, focus ties, and full `u32` extents.

Claude independently compiled an earlier 21-test version with stub pane/rectangle
types on stable Rust 1.99.1 and reported 21 passing tests. A later test adds the
opposite-divider regression. The independent check verifies the earlier pure logic;
the repository formatting, Clippy, and full-workspace results above are Codex's.

No platform-specific code changed. Native Mac rendering checks remain pending as
listed in the Phase 0 report. This phase does not change runtime pane behavior.

## Integration notes for the next phases

- Remove the temporary non-test `dead_code` allowance when wiring layout into
  native windows in Phase 2.
- Derive `MinSize` from shared font metrics plus padding. Use the tree minimum to
  constrain native-window size; defensively handle `TooSmall` by preserving valid
  pane geometry and clipping if the platform still forces a smaller window.
  Update the GL surface to the actual native size even in that fallback.
- Use one window begin-frame, all pane draws, then one swap/frame request; use
  full-window damage initially for multiple panes.
- A divider drag takes its leading-edge X/Y. Phase 3 must retain/subtract the
  pointer's grab offset to prevent jumping when the drag starts.
- Keep the diagonal focus fallback: a 2px-tall pane can face an opposite 2px
  divider across its entire height in a valid tiling. The new regression test
  covers this geometry and deterministic selection of a diagonal neighbor.

Stop at the Phase 1 review checkpoint before implementing native splits.
