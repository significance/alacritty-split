//! Pure split-tree layout. All bounds and divider positions are integer physical pixels.

use std::cmp::Reverse;
use std::collections::BTreeMap;

use super::PaneId;
use super::geometry::PaneRect;

/// Vertical dividers create side-by-side panes; horizontal dividers create stacked panes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusDirection {
    Left,
    Right,
    Up,
    Down,
}

/// The caller derives this from cell metrics and padding, independently of the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MinSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayoutParams {
    pub bounds: PaneRect,
    pub divider: u32,
    pub minimum: MinSize,
}

/// Exact rational weight; the first child gets the floor and the second gets the remainder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplitRatio {
    numerator: u32,
    denominator: u32,
}

impl SplitRatio {
    const HALF: Self = Self { numerator: 1, denominator: 2 };

    fn from_pixels(first: u32, available: u32) -> Self {
        debug_assert!(first > 0 && first < available);
        let (mut a, mut b) = (first, available);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        Self { numerator: first / a, denominator: available / a }
    }

    fn pixels(self, available: u32) -> u32 {
        (u64::from(available) * u64::from(self.numerator) / u64::from(self.denominator)) as u32
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Layout {
    Leaf(PaneId),
    Split { orientation: Orientation, ratio: SplitRatio, first: Box<Layout>, second: Box<Layout> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Child {
    First,
    Second,
}

/// A divider's path is valid until the tree structure changes; recompute after split/close.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SplitPath(pub Vec<Child>);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Divider {
    pub path: SplitPath,
    pub orientation: Orientation,
    pub rect: PaneRect,
    /// Parent bounds are needed to translate a window-space drag into a ratio.
    pub bounds: PaneRect,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayoutGeometry {
    pub panes: BTreeMap<PaneId, PaneRect>,
    pub dividers: Vec<Divider>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutError {
    MissingPane(PaneId),
    DuplicatePane(PaneId),
    LastPane,
    TooSmall,
    InvalidMinimum,
    InvalidBounds,
    MissingSplit,
}

impl Layout {
    /// Rebuild a fallback tree while retaining every surviving terminal identity.
    pub fn from_panes(panes: &[PaneId]) -> Self {
        assert!(!panes.is_empty());
        let mut tree = Self::Leaf(panes[0]);
        for &id in &panes[1..] {
            tree = Self::Split {
                orientation: Orientation::Vertical,
                ratio: SplitRatio::HALF,
                first: Box::new(tree),
                second: Box::new(Self::Leaf(id)),
            };
        }
        tree
    }

    /// Remove a dead leaf even if current physical geometry cannot be computed.
    pub fn discard(&mut self, pane: PaneId) -> Result<PaneId, LayoutError> {
        if let Self::Leaf(id) = self {
            return Err(if *id == pane {
                LayoutError::LastPane
            } else {
                LayoutError::MissingPane(pane)
            });
        }
        let siblings = self.remove_leaf(pane).ok_or(LayoutError::MissingPane(pane))?;
        Ok(siblings[0])
    }

    /// Compute a resize without mutating weights, even when the window is too small.
    pub fn geometry(&self, params: LayoutParams) -> Result<LayoutGeometry, LayoutError> {
        let bounds = params.bounds;
        if params.minimum.width == 0 || params.minimum.height == 0 {
            return Err(LayoutError::InvalidMinimum);
        }
        if bounds.x.checked_add(bounds.width).is_none()
            || bounds.y.checked_add(bounds.height).is_none()
        {
            return Err(LayoutError::InvalidBounds);
        }
        let minimum = self.minimum(params)?;
        if bounds.width < minimum.width || bounds.height < minimum.height {
            return Err(LayoutError::TooSmall);
        }
        let mut result = LayoutGeometry::default();
        self.place(bounds, params, &mut Vec::new(), &mut result)?;
        Ok(result)
    }

    /// Replace only the target leaf with a half split, committing only after validation.
    pub fn split(
        &mut self,
        pane: PaneId,
        new_pane: PaneId,
        orientation: Orientation,
        params: LayoutParams,
    ) -> Result<LayoutGeometry, LayoutError> {
        let geometry = self.geometry(params)?;
        if geometry.panes.contains_key(&new_pane) {
            return Err(LayoutError::DuplicatePane(new_pane));
        }
        let rect = *geometry.panes.get(&pane).ok_or(LayoutError::MissingPane(pane))?;
        // A split must fit inside this pane; never steal space from its neighbors.
        let needed = combined_minimum(params.minimum, params.minimum, orientation, params.divider)?;
        if rect.width < needed.width || rect.height < needed.height {
            return Err(LayoutError::TooSmall);
        }
        let mut candidate = self.clone();
        let leaf = candidate.leaf_mut(pane).ok_or(LayoutError::MissingPane(pane))?;
        *leaf = Self::Split {
            orientation,
            ratio: SplitRatio::HALF,
            first: Box::new(Self::Leaf(pane)),
            second: Box::new(Self::Leaf(new_pane)),
        };
        let geometry = candidate.geometry(params)?;
        *self = candidate;
        Ok(geometry)
    }

    /// Collapse the removed pane's parent; focus the nearest leaf in its sibling subtree.
    pub fn remove(&mut self, pane: PaneId, params: LayoutParams) -> Result<PaneId, LayoutError> {
        let geometry = self.geometry(params)?;
        let closed = *geometry.panes.get(&pane).ok_or(LayoutError::MissingPane(pane))?;
        if geometry.panes.len() == 1 {
            return Err(LayoutError::LastPane);
        }
        let mut candidate = self.clone();
        let siblings = candidate.remove_leaf(pane).ok_or(LayoutError::MissingPane(pane))?;
        let next = siblings
            .into_iter()
            .min_by_key(|id| {
                let rect = geometry.panes[id];
                (rect_distance(closed, rect), center_distance(closed, rect), *id)
            })
            .expect("a sibling subtree contains a leaf");
        candidate.geometry(params)?;
        *self = candidate;
        Ok(next)
    }

    /// Move the divider to an absolute window-space X/Y, clamped by both subtree minima.
    pub fn drag_split(
        &mut self,
        path: &SplitPath,
        position: u32,
        params: LayoutParams,
    ) -> Result<LayoutGeometry, LayoutError> {
        let geometry = self.geometry(params)?;
        let divider = geometry
            .dividers
            .iter()
            .find(|divider| divider.path == *path)
            .ok_or(LayoutError::MissingSplit)?;
        let mut candidate = self.clone();
        let node = candidate.node_mut(&path.0).ok_or(LayoutError::MissingSplit)?;
        if let Self::Split { orientation, ratio, first, second } = node {
            let first_min = first.minimum(params)?;
            let second_min = second.minimum(params)?;
            let (start, extent, low, second_min) = match orientation {
                Orientation::Vertical => {
                    (divider.bounds.x, divider.bounds.width, first_min.width, second_min.width)
                },
                Orientation::Horizontal => {
                    (divider.bounds.y, divider.bounds.height, first_min.height, second_min.height)
                },
            };
            let available = extent - params.divider;
            let first = position.saturating_sub(start).clamp(low, available - second_min);
            *ratio = SplitRatio::from_pixels(first, available);
        } else {
            return Err(LayoutError::MissingSplit);
        }
        let geometry = candidate.geometry(params)?;
        *self = candidate;
        Ok(geometry)
    }

    /// Minimum physical bounds required by every leaf and divider.
    pub fn minimum(&self, params: LayoutParams) -> Result<MinSize, LayoutError> {
        if params.minimum.width == 0 || params.minimum.height == 0 {
            return Err(LayoutError::InvalidMinimum);
        }
        match self {
            Self::Leaf(_) => Ok(params.minimum),
            Self::Split { orientation, first, second, .. } => combined_minimum(
                first.minimum(params)?,
                second.minimum(params)?,
                *orientation,
                params.divider,
            ),
        }
    }

    fn place(
        &self,
        bounds: PaneRect,
        params: LayoutParams,
        path: &mut Vec<Child>,
        result: &mut LayoutGeometry,
    ) -> Result<(), LayoutError> {
        match self {
            Self::Leaf(id) => {
                if result.panes.insert(*id, bounds).is_some() {
                    return Err(LayoutError::DuplicatePane(*id));
                }
            },
            Self::Split { orientation, ratio, first, second } => {
                let first_min = first.minimum(params)?;
                let second_min = second.minimum(params)?;
                let (extent, low, second_min) = match orientation {
                    Orientation::Vertical => (bounds.width, first_min.width, second_min.width),
                    Orientation::Horizontal => (bounds.height, first_min.height, second_min.height),
                };
                let available = extent - params.divider;
                let first_size = ratio.pixels(available).clamp(low, available - second_min);
                let (first_rect, divider_rect, second_rect) = match orientation {
                    Orientation::Vertical => (
                        PaneRect { width: first_size, ..bounds },
                        PaneRect { x: bounds.x + first_size, width: params.divider, ..bounds },
                        PaneRect {
                            x: bounds.x + first_size + params.divider,
                            width: available - first_size,
                            ..bounds
                        },
                    ),
                    Orientation::Horizontal => (
                        PaneRect { height: first_size, ..bounds },
                        PaneRect { y: bounds.y + first_size, height: params.divider, ..bounds },
                        PaneRect {
                            y: bounds.y + first_size + params.divider,
                            height: available - first_size,
                            ..bounds
                        },
                    ),
                };
                result.dividers.push(Divider {
                    path: SplitPath(path.clone()),
                    orientation: *orientation,
                    rect: divider_rect,
                    bounds,
                });
                path.push(Child::First);
                first.place(first_rect, params, path, result)?;
                path.pop();
                path.push(Child::Second);
                second.place(second_rect, params, path, result)?;
                path.pop();
            },
        }
        Ok(())
    }

    fn leaf_mut(&mut self, pane: PaneId) -> Option<&mut Self> {
        match self {
            Self::Leaf(id) if *id == pane => Some(self),
            Self::Split { first, second, .. } => {
                first.leaf_mut(pane).or_else(|| second.leaf_mut(pane))
            },
            _ => None,
        }
    }

    fn node_mut(&mut self, path: &[Child]) -> Option<&mut Self> {
        let Some((child, rest)) = path.split_first() else {
            return Some(self);
        };
        match (self, child) {
            (Self::Split { first, .. }, Child::First) => first.node_mut(rest),
            (Self::Split { second, .. }, Child::Second) => second.node_mut(rest),
            _ => None,
        }
    }

    fn leaf_ids(&self, ids: &mut Vec<PaneId>) {
        match self {
            Self::Leaf(id) => ids.push(*id),
            Self::Split { first, second, .. } => {
                first.leaf_ids(ids);
                second.leaf_ids(ids);
            },
        }
    }

    fn remove_leaf(&mut self, pane: PaneId) -> Option<Vec<PaneId>> {
        if let Self::Split { first, second, .. } = self {
            let sibling = if **first == Self::Leaf(pane) {
                Some(&**second)
            } else if **second == Self::Leaf(pane) {
                Some(&**first)
            } else {
                None
            };
            if let Some(sibling) = sibling {
                let mut ids = Vec::new();
                sibling.leaf_ids(&mut ids);
                *self = sibling.clone();
                return Some(ids);
            }
            return first.remove_leaf(pane).or_else(|| second.remove_leaf(pane));
        }
        None
    }
}

fn combined_minimum(
    first: MinSize,
    second: MinSize,
    orientation: Orientation,
    divider: u32,
) -> Result<MinSize, LayoutError> {
    let sum = |a: u32, b: u32| {
        a.checked_add(b).and_then(|sum| sum.checked_add(divider)).ok_or(LayoutError::TooSmall)
    };
    match orientation {
        Orientation::Vertical => Ok(MinSize {
            width: sum(first.width, second.width)?,
            height: first.height.max(second.height),
        }),
        Orientation::Horizontal => Ok(MinSize {
            width: first.width.max(second.width),
            height: sum(first.height, second.height)?,
        }),
    }
}

impl LayoutGeometry {
    /// Prefer overlapping edges, then axis gap and alignment with the focused pane's center.
    /// Equal geometry is resolved by overlap, center distance and finally lowest PaneId.
    pub fn neighbor(&self, pane: PaneId, direction: FocusDirection) -> Option<PaneId> {
        let from = *self.panes.get(&pane)?;
        self.panes
            .iter()
            .filter(|(id, _)| **id != pane)
            .filter_map(|(&id, &to)| {
                let (axis_from, axis_to, cross_from, cross_to) = match direction {
                    FocusDirection::Left | FocusDirection::Right => {
                        (x_interval(from), x_interval(to), y_interval(from), y_interval(to))
                    },
                    FocusDirection::Up | FocusDirection::Down => {
                        (y_interval(from), y_interval(to), x_interval(from), x_interval(to))
                    },
                };
                let gap = match direction {
                    FocusDirection::Right | FocusDirection::Down if axis_to.0 >= axis_from.1 => {
                        axis_to.0 - axis_from.1
                    },
                    FocusDirection::Left | FocusDirection::Up if axis_to.1 <= axis_from.0 => {
                        axis_from.0 - axis_to.1
                    },
                    _ => return None,
                };
                let overlap =
                    cross_from.1.min(cross_to.1).saturating_sub(cross_from.0.max(cross_to.0));
                let center = cross_from.0 + cross_from.1;
                let alignment = (cross_to.0 * 2)
                    .saturating_sub(center)
                    .max(center.saturating_sub(cross_to.1 * 2));
                Some((
                    (overlap == 0, gap, alignment, Reverse(overlap), center_distance(from, to), id),
                    id,
                ))
            })
            .min_by_key(|(score, _)| *score)
            .map(|(_, id)| id)
    }
}

fn x_interval(rect: PaneRect) -> (u64, u64) {
    (u64::from(rect.x), u64::from(rect.x) + u64::from(rect.width))
}

fn y_interval(rect: PaneRect) -> (u64, u64) {
    (u64::from(rect.y), u64::from(rect.y) + u64::from(rect.height))
}

fn center_distance(a: PaneRect, b: PaneRect) -> u128 {
    let dx =
        (u64::from(a.x) * 2 + u64::from(a.width)).abs_diff(u64::from(b.x) * 2 + u64::from(b.width));
    let dy = (u64::from(a.y) * 2 + u64::from(a.height))
        .abs_diff(u64::from(b.y) * 2 + u64::from(b.height));
    u128::from(dx).pow(2) + u128::from(dy).pow(2)
}

fn rect_distance(a: PaneRect, b: PaneRect) -> u128 {
    let gap = |a: (u64, u64), b: (u64, u64)| a.0.saturating_sub(b.1).max(b.0.saturating_sub(a.1));
    u128::from(gap(x_interval(a), x_interval(b))).pow(2)
        + u128::from(gap(y_interval(a), y_interval(b))).pow(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(width: u32, height: u32) -> LayoutParams {
        LayoutParams {
            bounds: PaneRect { x: 0, y: 0, width, height },
            divider: 2,
            minimum: MinSize { width: 2, height: 2 },
        }
    }

    fn split(
        tree: &mut Layout,
        old: u64,
        new: u64,
        orientation: Orientation,
        params: LayoutParams,
    ) {
        tree.split(PaneId(old), PaneId(new), orientation, params).unwrap();
    }

    fn assert_partition(geometry: &LayoutGeometry, params: LayoutParams) {
        let bounds = params.bounds;
        let mut rectangles: Vec<_> = geometry.panes.values().copied().collect();
        assert!(!rectangles.is_empty());
        assert_eq!(geometry.dividers.len() + 1, rectangles.len());
        for rect in &rectangles {
            assert!(rect.width >= params.minimum.width);
            assert!(rect.height >= params.minimum.height);
        }
        for divider in &geometry.dividers {
            match divider.orientation {
                Orientation::Vertical => assert_eq!(divider.rect.width, params.divider),
                Orientation::Horizontal => assert_eq!(divider.rect.height, params.divider),
            }
            rectangles.push(divider.rect);
        }
        let mut area = 0_u128;
        for (index, &rect) in rectangles.iter().enumerate() {
            let (x, y) = (x_interval(rect), y_interval(rect));
            assert!(x.0 >= u64::from(bounds.x) && x.1 <= x_interval(bounds).1);
            assert!(y.0 >= u64::from(bounds.y) && y.1 <= y_interval(bounds).1);
            area += u128::from(rect.width) * u128::from(rect.height);
            if rect.width == 0 || rect.height == 0 {
                continue;
            }
            for &other in &rectangles[..index] {
                if other.width == 0 || other.height == 0 {
                    continue;
                }
                let (other_x, other_y) = (x_interval(other), y_interval(other));
                assert!(
                    x.1 <= other_x.0 || other_x.1 <= x.0 || y.1 <= other_y.0 || other_y.1 <= y.0,
                    "overlap: {rect:?} and {other:?}"
                );
            }
        }
        // In-bounds disjoint integer rectangles with exactly the window's area prove full cover.
        assert_eq!(area, u128::from(bounds.width) * u128::from(bounds.height));
    }

    #[test]
    fn middle_exit_preserves_partition_and_focus_even_with_invalid_resize_geometry() {
        let p = params(300, 200);
        let mut tree = Layout::Leaf(PaneId(0));
        split(&mut tree, 0, 1, Orientation::Vertical, p);
        split(&mut tree, 1, 2, Orientation::Horizontal, p);
        assert_eq!(tree.remove(PaneId(1), p), Ok(PaneId(2)));
        let geometry = tree.geometry(p).unwrap();
        assert_partition(&geometry, p);
        assert_eq!(geometry.neighbor(PaneId(2), FocusDirection::Left), Some(PaneId(0)));
        let invalid = params(0, 0);
        assert_eq!(tree.remove(PaneId(2), invalid), Err(LayoutError::TooSmall));
        assert_eq!(tree.discard(PaneId(2)), Ok(PaneId(0)));
        assert_partition(&tree.geometry(p).unwrap(), p);
        let rebuilt = Layout::from_panes(&[PaneId(2), PaneId(5), PaneId(9)]);
        assert_partition(&rebuilt.geometry(p).unwrap(), p);
        assert_eq!(rebuilt.geometry(p).unwrap().panes.keys().copied().collect::<Vec<_>>(), [
            PaneId(2),
            PaneId(5),
            PaneId(9)
        ]);
    }

    #[test]
    fn leaf_fills_offset_window_without_dividers() {
        let mut p = params(101, 79);
        p.bounds.x = 11;
        p.bounds.y = 23;
        let geometry = Layout::Leaf(PaneId(9)).geometry(p).unwrap();
        assert_eq!(geometry.panes, [(PaneId(9), p.bounds)].into());
        assert_partition(&geometry, p);
    }

    #[test]
    fn vertical_split_gives_floor_to_existing_pane_and_remainder_to_right() {
        let p = params(11, 9);
        let mut tree = Layout::Leaf(PaneId(0));
        let geometry = tree.split(PaneId(0), PaneId(1), Orientation::Vertical, p).unwrap();
        assert_eq!(geometry.panes[&PaneId(0)], PaneRect { x: 0, y: 0, width: 4, height: 9 });
        assert_eq!(geometry.panes[&PaneId(1)], PaneRect { x: 6, y: 0, width: 5, height: 9 });
        assert_eq!(geometry.dividers[0].rect, PaneRect { x: 4, y: 0, width: 2, height: 9 });
        assert_partition(&geometry, p);
    }

    #[test]
    fn horizontal_split_puts_new_pane_below() {
        let p = params(9, 11);
        let mut tree = Layout::Leaf(PaneId(0));
        let geometry = tree.split(PaneId(0), PaneId(1), Orientation::Horizontal, p).unwrap();
        assert_eq!(geometry.panes[&PaneId(0)], PaneRect { x: 0, y: 0, width: 9, height: 4 });
        assert_eq!(geometry.panes[&PaneId(1)], PaneRect { x: 0, y: 6, width: 9, height: 5 });
        assert_partition(&geometry, p);
    }

    #[test]
    fn mixed_nested_splits_change_only_the_target_rectangle() {
        let mut p = params(31, 25);
        p.divider = 1;
        let mut tree = Layout::Leaf(PaneId(0));
        split(&mut tree, 0, 1, Orientation::Vertical, p);
        let first = tree.geometry(p).unwrap();
        split(&mut tree, 1, 2, Orientation::Horizontal, p);
        assert_eq!(tree.geometry(p).unwrap().panes[&PaneId(0)], first.panes[&PaneId(0)]);
        let before = tree.geometry(p).unwrap();
        split(&mut tree, 0, 3, Orientation::Vertical, p);
        let geometry = tree.geometry(p).unwrap();
        assert_eq!(geometry.panes[&PaneId(1)], before.panes[&PaneId(1)]);
        assert_eq!(geometry.panes[&PaneId(2)], before.panes[&PaneId(2)]);
        assert_eq!(geometry.panes[&PaneId(0)], PaneRect { x: 0, y: 0, width: 7, height: 25 });
        assert_eq!(geometry.panes[&PaneId(3)], PaneRect { x: 8, y: 0, width: 7, height: 25 });
        assert_partition(&geometry, p);
    }

    #[test]
    fn zero_width_dividers_and_shared_edges_still_cover_every_pixel() {
        let mut p = params(11, 11);
        p.divider = 0;
        let mut tree = Layout::Leaf(PaneId(0));
        split(&mut tree, 0, 1, Orientation::Vertical, p);
        split(&mut tree, 1, 2, Orientation::Horizontal, p);
        let geometry = tree.geometry(p).unwrap();
        assert_eq!(geometry.panes[&PaneId(0)].width, 5);
        assert_eq!(geometry.panes[&PaneId(1)].x, 5);
        assert_partition(&geometry, p);
    }

    #[test]
    fn exact_minimum_split_succeeds_one_pixel_less_is_atomic_error() {
        let mut p = params(22, 10);
        p.minimum = MinSize { width: 10, height: 10 };
        let mut tree = Layout::Leaf(PaneId(0));
        let geometry = tree.split(PaneId(0), PaneId(1), Orientation::Vertical, p).unwrap();
        assert_eq!(geometry.panes[&PaneId(0)].width, 10);
        assert_eq!(geometry.panes[&PaneId(1)].width, 10);
        assert_partition(&geometry, p);
        let mut tree = Layout::Leaf(PaneId(0));
        let before = tree.clone();
        p.bounds.width -= 1;
        assert_eq!(
            tree.split(PaneId(0), PaneId(1), Orientation::Vertical, p),
            Err(LayoutError::TooSmall)
        );
        assert_eq!(tree, before);
    }

    #[test]
    fn failed_split_cannot_steal_space_from_neighbor() {
        let mut p = params(62, 20);
        p.minimum = MinSize { width: 20, height: 2 };
        let mut tree = Layout::Leaf(PaneId(0));
        split(&mut tree, 0, 1, Orientation::Vertical, p);
        tree.drag_split(&SplitPath::default(), 20, p).unwrap();
        // There is enough room in the entire window for three minimum-sized leaves,
        // but not inside the focused 20px pane. Its neighbor must not move.
        let before = tree.clone();
        assert_eq!(
            tree.split(PaneId(0), PaneId(2), Orientation::Vertical, p),
            Err(LayoutError::TooSmall)
        );
        assert_eq!(tree, before);
    }

    #[test]
    fn duplicate_and_missing_pane_splits_leave_tree_unchanged() {
        let p = params(100, 80);
        let mut tree = Layout::Leaf(PaneId(0));
        split(&mut tree, 0, 1, Orientation::Vertical, p);
        let before = tree.clone();
        assert_eq!(
            tree.split(PaneId(0), PaneId(1), Orientation::Horizontal, p),
            Err(LayoutError::DuplicatePane(PaneId(1)))
        );
        assert_eq!(tree, before);
        assert_eq!(
            tree.split(PaneId(99), PaneId(2), Orientation::Horizontal, p),
            Err(LayoutError::MissingPane(PaneId(99)))
        );
        assert_eq!(tree, before);
    }

    #[test]
    fn resize_clamps_actual_rectangles_without_discarding_user_ratio() {
        let p = params(102, 40);
        let mut tree = Layout::Leaf(PaneId(0));
        split(&mut tree, 0, 1, Orientation::Vertical, p);
        tree.drag_split(&SplitPath::default(), 90, p).unwrap();
        let before = tree.clone();
        let mut small = p;
        small.bounds.width = 12;
        let geometry = tree.geometry(small).unwrap();
        assert_eq!(geometry.panes[&PaneId(0)].width, 8);
        assert_eq!(geometry.panes[&PaneId(1)].width, 2);
        assert_eq!(tree, before);
        assert_partition(&geometry, small);
        assert_eq!(tree.geometry(p).unwrap().panes[&PaneId(0)].width, 90);
        small.bounds.width = 5;
        assert_eq!(tree.geometry(small), Err(LayoutError::TooSmall));
        assert_eq!(tree, before);
    }

    #[test]
    fn tiny_invalid_and_overflowing_bounds_are_explicit_errors() {
        let mut p = params(1, 1);
        let tree = Layout::Leaf(PaneId(0));
        assert_eq!(tree.geometry(p), Err(LayoutError::TooSmall));
        p.minimum = MinSize { width: 1, height: 1 };
        assert_partition(&tree.geometry(p).unwrap(), p);
        p.bounds.width = 0;
        assert_eq!(tree.geometry(p), Err(LayoutError::TooSmall));
        p.bounds.width = 1;
        p.minimum.width = 0;
        assert_eq!(tree.geometry(p), Err(LayoutError::InvalidMinimum));
        p.minimum.width = 1;
        p.bounds.x = u32::MAX;
        assert_eq!(tree.geometry(p), Err(LayoutError::InvalidBounds));
    }

    #[test]
    fn large_coordinates_use_wide_intermediate_math() {
        let mut p = params(u32::MAX, u32::MAX);
        p.minimum = MinSize { width: 1, height: 1 };
        let mut tree = Layout::Leaf(PaneId(0));
        split(&mut tree, 0, 1, Orientation::Vertical, p);
        let geometry = tree.drag_split(&SplitPath::default(), u32::MAX, p).unwrap();
        assert_eq!(geometry.panes[&PaneId(1)].width, 1);
        assert_partition(&geometry, p);
        assert_eq!(geometry.neighbor(PaneId(0), FocusDirection::Right), Some(PaneId(1)));
        let before = tree.clone();
        p.minimum.width = u32::MAX;
        assert_eq!(tree.geometry(p), Err(LayoutError::TooSmall));
        assert_eq!(tree, before);
    }

    #[test]
    fn dragging_clamps_against_entire_nested_subtrees() {
        let mut p = params(120, 40);
        p.minimum = MinSize { width: 20, height: 10 };
        let mut tree = Layout::Leaf(PaneId(0));
        split(&mut tree, 0, 1, Orientation::Vertical, p);
        split(&mut tree, 0, 2, Orientation::Vertical, p);
        let root = SplitPath::default();
        let geometry = tree.drag_split(&root, 0, p).unwrap();
        assert_eq!(geometry.panes[&PaneId(0)].width, 20);
        assert_eq!(geometry.panes[&PaneId(2)].width, 20);
        assert_eq!(geometry.panes[&PaneId(1)].x, 44);
        assert_partition(&geometry, p);
        let geometry = tree.drag_split(&root, u32::MAX, p).unwrap();
        assert_eq!(geometry.panes[&PaneId(1)].width, 20);
        assert_partition(&geometry, p);
        let left = SplitPath(vec![Child::First]);
        let geometry = tree.drag_split(&left, u32::MAX, p).unwrap();
        assert_eq!(geometry.panes[&PaneId(2)].width, 20);
        assert_partition(&geometry, p);
    }

    #[test]
    fn dragging_nested_horizontal_divider_uses_absolute_window_coordinate() {
        let mut p = params(80, 102);
        p.bounds.x = 10;
        p.bounds.y = 20;
        let mut tree = Layout::Leaf(PaneId(0));
        split(&mut tree, 0, 1, Orientation::Horizontal, p);
        split(&mut tree, 1, 2, Orientation::Horizontal, p);
        let path = SplitPath(vec![Child::Second]);
        let geometry = tree.drag_split(&path, 80, p).unwrap();
        assert_eq!(geometry.dividers.iter().find(|d| d.path == path).unwrap().rect.y, 80);
        assert_eq!(geometry.panes[&PaneId(0)].height, 50);
        assert_partition(&geometry, p);
    }

    #[test]
    fn failed_drag_and_resize_leave_ratios_and_structure_unchanged() {
        let p = params(100, 80);
        let mut tree = Layout::Leaf(PaneId(0));
        split(&mut tree, 0, 1, Orientation::Vertical, p);
        let before = tree.clone();
        assert_eq!(
            tree.drag_split(&SplitPath(vec![Child::First]), 10, p),
            Err(LayoutError::MissingSplit)
        );
        assert_eq!(tree, before);
        let too_small = LayoutParams { bounds: PaneRect { width: 1, ..p.bounds }, ..p };
        assert_eq!(
            tree.drag_split(&SplitPath::default(), 10, too_small),
            Err(LayoutError::TooSmall)
        );
        assert_eq!(tree, before);
        assert_eq!(tree.geometry(too_small), Err(LayoutError::TooSmall));
        assert_eq!(tree, before);
    }

    #[test]
    fn removal_collapses_parent_and_returns_focus_from_sibling_only() {
        let p = params(102, 102);
        let mut tree = Layout::Leaf(PaneId(0));
        split(&mut tree, 0, 1, Orientation::Vertical, p);
        split(&mut tree, 0, 2, Orientation::Horizontal, p);
        // Closing top-left must select its bottom-left sibling, never the right pane.
        assert_eq!(tree.remove(PaneId(0), p), Ok(PaneId(2)));
        let geometry = tree.geometry(p).unwrap();
        assert_eq!(geometry.panes[&PaneId(2)].height, 102);
        assert!(!geometry.panes.contains_key(&PaneId(0)));
        assert_partition(&geometry, p);
        assert_eq!(tree.remove(PaneId(1), p), Ok(PaneId(2)));
        assert_eq!(tree, Layout::Leaf(PaneId(2)));
    }

    #[test]
    fn removal_selects_nearest_sibling_leaf_with_deterministic_ties() {
        let p = params(102, 102);
        let mut tree = Layout::Leaf(PaneId(0));
        split(&mut tree, 0, 9, Orientation::Vertical, p);
        split(&mut tree, 9, 1, Orientation::Horizontal, p);
        let mut tied = tree.clone();
        // Both equally distant: lowest identity wins, regardless of traversal order.
        assert_eq!(tied.remove(PaneId(0), p), Ok(PaneId(1)));
        // Most of the sibling region is above; geometry should win over the lower ID.
        tree.drag_split(&SplitPath(vec![Child::Second]), 80, p).unwrap();
        assert_eq!(tree.remove(PaneId(0), p), Ok(PaneId(9)));
        assert_partition(&tree.geometry(p).unwrap(), p);
    }

    #[test]
    fn removing_missing_or_last_pane_is_atomic_error() {
        let p = params(100, 80);
        let mut tree = Layout::Leaf(PaneId(7));
        let before = tree.clone();
        assert_eq!(tree.remove(PaneId(7), p), Err(LayoutError::LastPane));
        assert_eq!(tree, before);
        assert_eq!(tree.remove(PaneId(0), p), Err(LayoutError::MissingPane(PaneId(0))));
        assert_eq!(tree, before);
    }

    #[test]
    fn directional_focus_uses_nested_geometry_and_lowest_id_for_ties() {
        let p = params(102, 102);
        let mut tree = Layout::Leaf(PaneId(0));
        split(&mut tree, 0, 9, Orientation::Vertical, p);
        split(&mut tree, 9, 1, Orientation::Horizontal, p);
        let geometry = tree.geometry(p).unwrap();
        assert_eq!(geometry.neighbor(PaneId(0), FocusDirection::Right), Some(PaneId(1)));
        assert_eq!(geometry.neighbor(PaneId(9), FocusDirection::Down), Some(PaneId(1)));
        assert_eq!(geometry.neighbor(PaneId(1), FocusDirection::Up), Some(PaneId(9)));
        assert_eq!(geometry.neighbor(PaneId(1), FocusDirection::Left), Some(PaneId(0)));
        assert_eq!(geometry.neighbor(PaneId(0), FocusDirection::Left), None);
        assert_eq!(geometry.neighbor(PaneId(9), FocusDirection::Up), None);
        assert_eq!(geometry.neighbor(PaneId(9), FocusDirection::Right), None);
        assert_eq!(geometry.neighbor(PaneId(99), FocusDirection::Right), None);
        tree.drag_split(&SplitPath(vec![Child::Second]), 80, p).unwrap();
        assert_eq!(
            tree.geometry(p).unwrap().neighbor(PaneId(0), FocusDirection::Right),
            Some(PaneId(9))
        );
    }

    #[test]
    fn focus_prefers_shared_edge_over_diagonal_center_distance() {
        // A pure geometry fixture isolates the policy from split ratios.
        let geometry = LayoutGeometry {
            panes: [
                (PaneId(0), PaneRect { x: 0, y: 0, width: 10, height: 10 }),
                (PaneId(1), PaneRect { x: 11, y: 11, width: 10, height: 10 }),
                (PaneId(2), PaneRect { x: 30, y: 0, width: 10, height: 10 }),
            ]
            .into(),
            dividers: Vec::new(),
        };
        assert_eq!(geometry.neighbor(PaneId(0), FocusDirection::Right), Some(PaneId(2)));
    }

    #[test]
    fn focus_can_need_diagonal_fallback_when_opposite_divider_covers_entire_pane() {
        let p = params(22, 22);
        let mut tree = Layout::Leaf(PaneId(0));
        split(&mut tree, 0, 1, Orientation::Vertical, p);
        split(&mut tree, 0, 2, Orientation::Horizontal, p);
        tree.drag_split(&SplitPath(vec![Child::First]), 8, p).unwrap();
        split(&mut tree, 2, 3, Orientation::Horizontal, p);
        tree.drag_split(&SplitPath(vec![Child::First, Child::Second]), 12, p).unwrap();
        split(&mut tree, 1, 4, Orientation::Horizontal, p);
        let geometry = tree.geometry(p).unwrap();
        assert_partition(&geometry, p);
        // The left pane occupies y=10..12, exactly opposite the right divider.
        assert_eq!(geometry.panes[&PaneId(2)], PaneRect { x: 0, y: 10, width: 10, height: 2 });
        assert_eq!(geometry.panes[&PaneId(1)].y + geometry.panes[&PaneId(1)].height, 10);
        assert_eq!(geometry.panes[&PaneId(4)].y, 12);
        // Both diagonal alternatives tie geometrically; choose the lowest PaneId.
        assert_eq!(geometry.neighbor(PaneId(2), FocusDirection::Right), Some(PaneId(1)));
    }

    #[test]
    fn invalid_duplicate_tree_is_rejected_without_overwriting_geometry() {
        let tree = Layout::Split {
            orientation: Orientation::Vertical,
            ratio: SplitRatio::HALF,
            first: Box::new(Layout::Leaf(PaneId(0))),
            second: Box::new(Layout::Leaf(PaneId(0))),
        };
        assert_eq!(tree.geometry(params(100, 80)), Err(LayoutError::DuplicatePane(PaneId(0))));
    }

    #[test]
    fn random_split_close_drag_resize_sequences_preserve_partition_and_atomicity() {
        let mut successes = [0; 4];
        let mut max_panes = 1;
        for seed in 0..32_u64 {
            let mut state = seed + 1;
            let mut random = || {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                (state >> 32) as u32
            };
            let mut p = params(96, 72);
            p.minimum = MinSize { width: 3, height: 2 };
            p.divider = 1;
            let mut tree = Layout::Leaf(PaneId(0));
            let mut next_id = 1;
            for _ in 0..384 {
                let before = tree.clone();
                let geometry = tree.geometry(p).unwrap();
                let ids: Vec<_> = geometry.panes.keys().copied().collect();
                let pane = ids[random() as usize % ids.len()];
                match random() % 4 {
                    0 if ids.len() < 6 => {
                        let new = PaneId(next_id);
                        next_id += 1;
                        let orientation = if random() % 2 == 0 {
                            Orientation::Vertical
                        } else {
                            Orientation::Horizontal
                        };
                        match tree.split(pane, new, orientation, p) {
                            Ok(geometry) => {
                                successes[0] += 1;
                                assert_eq!(geometry.panes.len(), ids.len() + 1);
                                assert!(geometry.panes.contains_key(&new));
                            },
                            Err(_) => assert_eq!(tree, before),
                        }
                    },
                    1 => match tree.remove(pane, p) {
                        Ok(next_focus) => {
                            successes[1] += 1;
                            let geometry = tree.geometry(p).unwrap();
                            assert!(!geometry.panes.contains_key(&pane));
                            assert!(geometry.panes.contains_key(&next_focus));
                        },
                        Err(_) => assert_eq!(tree, before),
                    },
                    2 => {
                        let candidate = LayoutParams {
                            bounds: PaneRect {
                                x: random() % 8,
                                y: random() % 8,
                                width: 1 + random() % 96,
                                height: 1 + random() % 72,
                            },
                            divider: random() % 3,
                            minimum: MinSize { width: 1 + random() % 4, height: 1 + random() % 3 },
                        };
                        match tree.geometry(candidate) {
                            Ok(geometry) => {
                                successes[2] += 1;
                                assert_partition(&geometry, candidate);
                                p = candidate;
                            },
                            Err(_) => assert_eq!(tree, before),
                        }
                        assert_eq!(tree, before);
                    },
                    _ if !geometry.dividers.is_empty() => {
                        let divider =
                            &geometry.dividers[random() as usize % geometry.dividers.len()];
                        match tree.drag_split(&divider.path, random() % 128, p) {
                            Ok(_) => successes[3] += 1,
                            Err(_) => assert_eq!(tree, before),
                        }
                    },
                    _ => (),
                }
                let geometry = tree.geometry(p).unwrap();
                assert!(geometry.panes.len() <= 6);
                max_panes = max_panes.max(geometry.panes.len());
                assert_partition(&geometry, p);
                for &id in geometry.panes.keys() {
                    for direction in [
                        FocusDirection::Left,
                        FocusDirection::Right,
                        FocusDirection::Up,
                        FocusDirection::Down,
                    ] {
                        if let Some(neighbor) = geometry.neighbor(id, direction) {
                            assert_ne!(neighbor, id);
                            assert!(geometry.panes.contains_key(&neighbor));
                        }
                    }
                }
            }
        }
        assert_eq!(max_panes, 6, "random sequences must exercise six-pane layouts");
        assert!(
            successes.iter().all(|&count| count > 50),
            "insufficient operation coverage: {successes:?}"
        );
    }
}
