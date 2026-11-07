//! Zoom is a presentation filter; the tiling tree and its ratios are never mutated.

use super::layout::{Layout, LayoutError, LayoutGeometry, LayoutParams};
use super::{PaneCommand, PaneId};

pub fn after_command(
    zoomed: Option<PaneId>,
    focused: PaneId,
    pane_count: usize,
    command: PaneCommand,
) -> Option<PaneId> {
    match command {
        PaneCommand::ToggleZoom if zoomed.is_some() => None,
        PaneCommand::ToggleZoom if pane_count > 1 => Some(focused),
        PaneCommand::ToggleZoom => None,
        PaneCommand::Split(_) | PaneCommand::Close | PaneCommand::Focus(_) => None,
    }
}

pub fn after_exit(zoomed: Option<PaneId>, exiting: PaneId) -> Option<PaneId> {
    zoomed.filter(|id| *id != exiting)
}

pub fn geometry(
    layout: &Layout,
    params: LayoutParams,
    zoomed: Option<PaneId>,
) -> Result<LayoutGeometry, LayoutError> {
    let tiling = layout.geometry(params)?;
    match zoomed {
        Some(id) if tiling.panes.contains_key(&id) => {
            Ok(LayoutGeometry { panes: [(id, params.bounds)].into(), dividers: Vec::new() })
        },
        Some(id) => Err(LayoutError::MissingPane(id)),
        None => Ok(tiling),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pane::geometry::PaneRect;
    use crate::pane::layout::{FocusDirection, MinSize, Orientation, SplitPath};

    fn params() -> LayoutParams {
        LayoutParams {
            bounds: PaneRect { x: 0, y: 0, width: 801, height: 600 },
            divider: 1,
            minimum: MinSize { width: 10, height: 10 },
        }
    }

    fn tree() -> Layout {
        let mut tree = Layout::Leaf(PaneId(0));
        tree.split(PaneId(0), PaneId(1), Orientation::Vertical, params()).unwrap();
        tree.split(PaneId(1), PaneId(2), Orientation::Horizontal, params()).unwrap();
        tree.drag_split(&SplitPath::default(), 315, params()).unwrap();
        tree
    }

    #[test]
    fn zoom_restores_exact_tree_ratios_order_and_geometry() {
        let tree = tree();
        let before = tree.clone();
        let original = geometry(&tree, params(), None).unwrap();
        let zoomed = after_command(None, PaneId(1), 3, PaneCommand::ToggleZoom);
        let visible = geometry(&tree, params(), zoomed).unwrap();
        assert_eq!(visible.panes, [(PaneId(1), params().bounds)].into());
        assert!(visible.dividers.is_empty());
        assert_eq!(tree, before);
        let restored = after_command(zoomed, PaneId(1), 3, PaneCommand::ToggleZoom);
        assert_eq!(geometry(&tree, params(), restored).unwrap(), original);
        assert_eq!(tree, before);
    }

    #[test]
    fn resize_updates_only_visible_rectangles_and_preserves_hidden_sizes_until_unzoom() {
        let tree = tree();
        let initial = geometry(&tree, params(), None).unwrap();
        let mut pane_rects = initial.panes.clone();
        let mut resized = params();
        resized.bounds.width = 1200;
        resized.bounds.height = 700;
        let visible = geometry(&tree, resized, Some(PaneId(1))).unwrap();
        for (id, rect) in visible.panes {
            pane_rects.insert(id, rect);
        }
        assert_eq!(pane_rects[&PaneId(0)], initial.panes[&PaneId(0)]);
        assert_eq!(pane_rects[&PaneId(2)], initial.panes[&PaneId(2)]);
        assert_eq!(pane_rects[&PaneId(1)], resized.bounds);
        for (id, rect) in geometry(&tree, resized, None).unwrap().panes {
            pane_rects.insert(id, rect);
        }
        assert_ne!(pane_rects[&PaneId(0)], initial.panes[&PaneId(0)]);
    }

    #[test]
    fn commands_leave_zoom_before_split_close_and_neighbor_focus() {
        let mut tree = tree();
        let zoomed = Some(PaneId(1));
        for command in [
            PaneCommand::Split(Orientation::Vertical),
            PaneCommand::Close,
            PaneCommand::Focus(FocusDirection::Left),
        ] {
            assert_eq!(after_command(zoomed, PaneId(1), 3, command), None);
        }
        let unzoomed =
            after_command(zoomed, PaneId(1), 3, PaneCommand::Split(Orientation::Vertical));
        tree.split(PaneId(1), PaneId(3), Orientation::Vertical, params()).unwrap();
        assert_eq!(geometry(&tree, params(), unzoomed).unwrap().panes.len(), 4);
        let unzoomed = after_command(zoomed, PaneId(1), 4, PaneCommand::Close);
        let focus = tree.remove(PaneId(1), params()).unwrap();
        assert!(!geometry(&tree, params(), unzoomed).unwrap().panes.contains_key(&PaneId(1)));
        assert_eq!(focus, PaneId(3));
    }

    #[test]
    fn hidden_exit_preserves_zoom_and_zoomed_exit_restores_survivors() {
        let mut tree = tree();
        let zoomed = Some(PaneId(1));
        let zoomed = after_exit(zoomed, PaneId(2));
        tree.remove(PaneId(2), params()).unwrap();
        assert_eq!(geometry(&tree, params(), zoomed).unwrap().panes.len(), 1);
        assert_eq!(zoomed, Some(PaneId(1)));
        let zoomed = after_exit(zoomed, PaneId(1));
        tree.remove(PaneId(1), params()).unwrap();
        assert_eq!(zoomed, None);
        assert_eq!(
            geometry(&tree, params(), zoomed).unwrap().panes.keys().copied().collect::<Vec<_>>(),
            [PaneId(0)]
        );
    }

    #[test]
    fn single_pane_toggle_is_a_no_op() {
        assert_eq!(after_command(None, PaneId(0), 1, PaneCommand::ToggleZoom), None);
        // If the last hidden shell exits, the surviving zoom can still be toggled off.
        assert_eq!(after_command(Some(PaneId(0)), PaneId(0), 1, PaneCommand::ToggleZoom), None);
    }
}
