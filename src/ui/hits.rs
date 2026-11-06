//! Clickable regions recorded while drawing, read back by mouse handling.

use ratatui::layout::{Position, Rect};

use crate::model::ResourceKind;

/// What a mouse position points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    ListRow(ResourceKind, usize),
    ListPane(ResourceKind),
    ListTab(ResourceKind),
    ViewerTab(usize),
    Viewer,
}

/// Screen regions from the most recent frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HitMap {
    /// One rect per visible list row: (row area, list, row index).
    pub rows: Vec<(Rect, ResourceKind, usize)>,
    /// Whole list panes.
    pub panes: Vec<(Rect, ResourceKind)>,
    /// List tabs in the narrow layout's tab strip.
    pub list_tabs: Vec<(Rect, ResourceKind)>,
    /// Viewer tab titles: (title area, tab index).
    pub tabs: Vec<(Rect, usize)>,
    /// The viewer pane.
    pub viewer: Rect,
}

impl HitMap {
    pub fn clear(&mut self) {
        *self = HitMap::default();
    }

    /// The most specific region under (`x`, `y`), if any.
    pub fn hit(&self, x: u16, y: u16) -> Option<Hit> {
        let p = Position::new(x, y);
        if let Some((_, kind, idx)) = self.rows.iter().find(|(r, ..)| r.contains(p)) {
            return Some(Hit::ListRow(*kind, *idx));
        }
        if let Some((_, kind)) = self.list_tabs.iter().find(|(r, _)| r.contains(p)) {
            return Some(Hit::ListTab(*kind));
        }
        if let Some((_, idx)) = self.tabs.iter().find(|(r, _)| r.contains(p)) {
            return Some(Hit::ViewerTab(*idx));
        }
        if let Some((_, kind)) = self.panes.iter().find(|(r, _)| r.contains(p)) {
            return Some(Hit::ListPane(*kind));
        }
        self.viewer.contains(p).then_some(Hit::Viewer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn most_specific_region_wins() {
        let map = HitMap {
            rows: vec![(Rect::new(1, 2, 10, 1), ResourceKind::Pods, 4)],
            panes: vec![(Rect::new(0, 0, 12, 8), ResourceKind::Pods)],
            tabs: vec![(Rect::new(21, 0, 4, 1), 1)],
            viewer: Rect::new(20, 0, 30, 8),
            ..HitMap::default()
        };
        assert_eq!(map.hit(3, 2), Some(Hit::ListRow(ResourceKind::Pods, 4)));
        assert_eq!(map.hit(3, 5), Some(Hit::ListPane(ResourceKind::Pods)));
        assert_eq!(map.hit(22, 0), Some(Hit::ViewerTab(1)));
        assert_eq!(map.hit(30, 4), Some(Hit::Viewer));
        assert_eq!(map.hit(15, 4), None);
        assert_eq!(HitMap::default().hit(0, 0), None);
    }
}
