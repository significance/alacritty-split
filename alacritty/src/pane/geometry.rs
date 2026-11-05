//! Pane bounds in native-window physical pixels.

/// Top-left origin and extent, independent of terminal cell dimensions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaneRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl PaneRect {
    /// OpenGL uses a bottom-left origin while window input uses top-left.
    pub fn gl_origin(self, window_height: u32) -> (i32, i32) {
        let x = i64::from(self.x);
        let y = i64::from(window_height) - i64::from(self.y) - i64::from(self.height);
        let to_gl = |value: i64| value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        (to_gl(x), to_gl(y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
