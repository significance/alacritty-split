//! Pane bounds in native-window physical pixels.

use super::PaneId;

/// Top-left origin and extent, independent of terminal cell dimensions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaneRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl PaneRect {
    /// Right and bottom edges are excluded so adjacent panes never both match.
    pub fn contains(self, position: (f64, f64)) -> bool {
        position.0 >= f64::from(self.x)
            && position.1 >= f64::from(self.y)
            && position.0 < f64::from(self.x) + f64::from(self.width)
            && position.1 < f64::from(self.y) + f64::from(self.height)
    }

    /// Preserve coordinates outside the pane for selection dragging/autoscrolling.
    pub fn local_position(self, position: (f64, f64)) -> (f64, f64) {
        (position.0 - f64::from(self.x), position.1 - f64::from(self.y))
    }

    /// OpenGL uses a bottom-left origin while window input uses top-left.
    pub fn gl_origin(self, window_height: u32) -> (i32, i32) {
        let x = i64::from(self.x);
        let y = i64::from(window_height) - i64::from(self.y) - i64::from(self.height);
        let to_gl = |value: i64| value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        (to_gl(x), to_gl(y))
    }
}

/// Find the pane under a window-space pointer without treating dividers as content.
pub fn hit_test(
    panes: impl IntoIterator<Item = (PaneId, PaneRect)>,
    position: (f64, f64),
) -> Option<PaneId> {
    panes.into_iter().find(|(_, rect)| rect.contains(position)).map(|(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_pane_pointer_mapping_is_identity() {
        let rect = PaneRect { x: 0, y: 0, width: 800, height: 600 };
        assert_eq!(hit_test([(PaneId(0), rect)], (12.5, 22.25)), Some(PaneId(0)));
        assert_eq!(rect.local_position((12.5, 22.25)), (12.5, 22.25));
        assert_eq!(rect.local_position((-2., 601.)), (-2., 601.));
    }

    #[test]
    fn hit_test_excludes_dividers_and_outer_edges() {
        let panes = [
            (PaneId(0), PaneRect { x: 0, y: 0, width: 400, height: 600 }),
            (PaneId(1), PaneRect { x: 402, y: 0, width: 398, height: 600 }),
        ];
        assert_eq!(hit_test(panes, (399.9, 0.)), Some(PaneId(0)));
        assert_eq!(hit_test(panes, (400., 20.)), None);
        assert_eq!(hit_test(panes, (401.9, 20.)), None);
        assert_eq!(hit_test(panes, (402., 20.)), Some(PaneId(1)));
        assert_eq!(hit_test(panes, (800., 20.)), None);
        assert_eq!(hit_test(panes, (10., 600.)), None);
    }

    #[test]
    fn local_coordinates_keep_fractional_and_outside_drag_positions() {
        let rect = PaneRect { x: 402, y: 302, width: 398, height: 298 };
        assert_eq!(rect.local_position((410.5, 318.25)), (8.5, 16.25));
        assert_eq!(rect.local_position((400., 300.)), (-2., -2.));
        assert_eq!(rect.local_position((802., 602.)), (400., 300.));
    }

    #[test]
    fn invalid_positions_and_empty_panes_are_not_hit() {
        let rect = PaneRect { x: 0, y: 0, width: 800, height: 600 };
        for position in [(-1., 1.), (1., -1.), (f64::NAN, 1.), (1., f64::INFINITY)] {
            assert_eq!(hit_test([(PaneId(0), rect)], position), None);
        }
        assert!(!PaneRect::default().contains((0., 0.)));
    }

    #[test]
    fn full_window_has_zero_gl_origin() {
        let rect = PaneRect { x: 0, y: 0, width: 800, height: 600 };
        assert_eq!(rect.gl_origin(600), (0, 0));
    }

    #[test]
    fn nested_panes_convert_top_left_to_bottom_left() {
        let rect = PaneRect { x: 402, y: 102, width: 398, height: 198 };
        assert_eq!(rect.gl_origin(600), (402, 300));
        let bottom = PaneRect { x: 402, y: 302, width: 398, height: 298 };
        assert_eq!(bottom.gl_origin(600), (402, 0));
    }

    #[test]
    fn shrinking_window_does_not_underflow_stale_rect() {
        let rect = PaneRect { x: 0, y: 302, width: 398, height: 298 };
        assert_eq!(rect.gl_origin(200), (0, -400));
        let huge = PaneRect { x: u32::MAX, y: u32::MAX, width: 1, height: u32::MAX };
        assert_eq!(huge.gl_origin(0), (i32::MAX, i32::MIN));
    }
}
