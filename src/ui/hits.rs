//! Clickable regions recorded while drawing, read back by mouse handling.

use ratatui::layout::Rect;

use crate::model::ResourceKind;

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
}
