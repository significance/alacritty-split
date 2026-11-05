//! Pointer ownership and divider gestures, independent of a native window.

use std::collections::HashSet;

use winit::event::{ElementState, MouseButton};

use super::PaneId;
use super::geometry::hit_test;
use super::layout::{LayoutGeometry, Orientation, SplitPath};

#[derive(Clone, Debug)]
pub struct DividerDrag {
    pub path: SplitPath,
    pub orientation: Orientation,
    grab_offset: f64,
}

impl DividerDrag {
    pub fn position(&self, pointer: (f64, f64)) -> u32 {
        let coordinate = match self.orientation {
            Orientation::Vertical => pointer.0,
            Orientation::Horizontal => pointer.1,
        };
        (coordinate - self.grab_offset).max(0.) as u32
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonAction {
    Forward,
    Focus(PaneId),
    Ignore,
}

#[derive(Default)]
pub struct PointerState {
    pub position: (f64, f64),
    pub drag: Option<DividerDrag>,
    pub capture: Option<PaneId>,
    pressed: HashSet<MouseButton>,
    suppressed: HashSet<MouseButton>,
}

impl PointerState {
    pub fn suppress(&mut self, button: MouseButton) {
        self.suppressed.insert(button);
    }

    pub fn has_suppressed_buttons(&self) -> bool {
        !self.suppressed.is_empty()
    }

    pub fn cancel_gesture(&mut self) {
        // Consume releases of buttons whose presses no longer belong to a pane.
        self.suppressed.extend(self.pressed.drain());
        self.capture = None;
        self.drag = None;
    }

    pub fn divider_at(&self, geometry: &LayoutGeometry) -> Option<Orientation> {
        geometry
            .dividers
            .iter()
            .find(|divider| divider.rect.contains(self.position))
            .map(|divider| divider.orientation)
    }

    pub fn button(
        &mut self,
        button: MouseButton,
        state: ElementState,
        focused: PaneId,
        geometry: &LayoutGeometry,
    ) -> ButtonAction {
        if state == ElementState::Released {
            if button == MouseButton::Left && self.drag.take().is_some() {
                self.suppressed.remove(&button);
                return ButtonAction::Ignore;
            }
            if self.suppressed.remove(&button) {
                return ButtonAction::Ignore;
            }
            self.pressed.remove(&button);
            if self.pressed.is_empty() {
                self.capture = None;
            }
            return ButtonAction::Forward;
        }
        if self.drag.is_some() {
            self.suppressed.insert(button);
            return ButtonAction::Ignore;
        }
        if self.capture.is_some() {
            self.pressed.insert(button);
            return ButtonAction::Forward;
        }
        if let Some(divider) =
            geometry.dividers.iter().find(|divider| divider.rect.contains(self.position))
        {
            self.suppressed.insert(button);
            if button == MouseButton::Left {
                let (coordinate, leading) = match divider.orientation {
                    Orientation::Vertical => (self.position.0, divider.rect.x),
                    Orientation::Horizontal => (self.position.1, divider.rect.y),
                };
                self.drag = Some(DividerDrag {
                    path: divider.path.clone(),
                    orientation: divider.orientation,
                    grab_offset: coordinate - f64::from(leading),
                });
            }
            return ButtonAction::Ignore;
        }
        match hit_test(geometry.panes.iter().map(|(&id, &rect)| (id, rect)), self.position) {
            Some(id) if id != focused => {
                self.suppressed.insert(button);
                ButtonAction::Focus(id)
            },
            Some(id) => {
                self.pressed.insert(button);
                self.capture = Some(id);
                ButtonAction::Forward
            },
            None => ButtonAction::Forward,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pane::geometry::PaneRect;
    use crate::pane::layout::{Layout, LayoutParams, MinSize};

    fn geometry() -> LayoutGeometry {
        let mut tree = Layout::Leaf(PaneId(0));
        tree.split(PaneId(0), PaneId(1), Orientation::Vertical, LayoutParams {
            bounds: PaneRect { x: 0, y: 0, width: 202, height: 100 },
            divider: 2,
            minimum: MinSize { width: 2, height: 2 },
        })
        .unwrap()
    }

    #[test]
    fn focus_click_consumes_press_and_release_then_allows_next_click() {
        let mut pointer = PointerState { position: (150., 20.), ..Default::default() };
        let geometry = geometry();
        assert_eq!(
            pointer.button(MouseButton::Left, ElementState::Pressed, PaneId(0), &geometry),
            ButtonAction::Focus(PaneId(1))
        );
        assert_eq!(
            pointer.button(MouseButton::Left, ElementState::Released, PaneId(1), &geometry),
            ButtonAction::Ignore
        );
        assert_eq!(
            pointer.button(MouseButton::Left, ElementState::Pressed, PaneId(1), &geometry),
            ButtonAction::Forward
        );
        assert_eq!(pointer.capture, Some(PaneId(1)));
    }

    #[test]
    fn divider_drag_preserves_grab_offset_and_consumes_both_edges() {
        let mut pointer = PointerState { position: (101.5, 20.), ..Default::default() };
        let geometry = geometry();
        assert_eq!(pointer.divider_at(&geometry), Some(Orientation::Vertical));
        assert_eq!(
            pointer.button(MouseButton::Left, ElementState::Pressed, PaneId(0), &geometry),
            ButtonAction::Ignore
        );
        assert_eq!(pointer.drag.as_ref().unwrap().position((151.5, 80.)), 150);
        assert_eq!(pointer.drag.as_ref().unwrap().position((-50., 80.)), 0);
        assert_eq!(
            pointer.button(MouseButton::Left, ElementState::Released, PaneId(0), &geometry),
            ButtonAction::Ignore
        );
        assert!(pointer.drag.is_none());
    }

    #[test]
    fn selection_capture_crosses_divider_and_focus_changes_consume_its_release() {
        let geometry = geometry();
        let mut pointer = PointerState { position: (20., 20.), ..Default::default() };
        assert_eq!(
            pointer.button(MouseButton::Left, ElementState::Pressed, PaneId(0), &geometry),
            ButtonAction::Forward
        );
        pointer.position = (150., 20.);
        assert_eq!(
            pointer.button(MouseButton::Right, ElementState::Pressed, PaneId(0), &geometry),
            ButtonAction::Forward
        );
        assert_eq!(pointer.capture, Some(PaneId(0)));
        pointer.button(MouseButton::Right, ElementState::Released, PaneId(0), &geometry);
        assert_eq!(pointer.capture, Some(PaneId(0)));
        pointer.cancel_gesture();
        assert_eq!(
            pointer.button(MouseButton::Left, ElementState::Released, PaneId(1), &geometry),
            ButtonAction::Ignore
        );
    }
}
