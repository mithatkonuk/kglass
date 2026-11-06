//! Screen regions for the wide and narrow layouts.

use ratatui::layout::{Constraint, Layout, Rect};

use crate::model::ResourceKind;

/// Minimum size for the wide layout (columns, rows).
pub const WIDE_MIN: (u16, u16) = (120, 30);
/// Below this size only a "too small" message is shown (columns, rows).
pub const MIN_SIZE: (u16, u16) = (60, 16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutMode {
    Wide,
    Narrow,
    TooSmall,
}

pub fn layout_mode(area: Rect) -> LayoutMode {
    if area.width < MIN_SIZE.0 || area.height < MIN_SIZE.1 {
        LayoutMode::TooSmall
    } else if area.width >= WIDE_MIN.0 && area.height >= WIDE_MIN.1 {
        LayoutMode::Wide
    } else {
        LayoutMode::Narrow
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Regions {
    pub header: Rect,
    pub footer: Rect,
    /// `[0] Context` box (wide, not zoomed).
    pub context: Option<Rect>,
    /// List tab strip (narrow, not zoomed).
    pub list_tabs: Option<Rect>,
    pub lists: Vec<(ResourceKind, Rect)>,
    pub viewer: Rect,
}

/// Split `area` for `mode`. `list_rows` is the row count of the list shown in
/// the narrow layout, used to size its pane.
pub fn regions(
    area: Rect,
    mode: LayoutMode,
    visible_list: ResourceKind,
    list_rows: usize,
    zoom: bool,
) -> Regions {
    if mode == LayoutMode::TooSmall {
        return Regions::default();
    }
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let mut r = Regions {
        header,
        footer,
        ..Regions::default()
    };
    if zoom {
        r.viewer = body;
        return r;
    }
    match mode {
        LayoutMode::Wide => {
            let left_width =
                u16::try_from((u32::from(area.width) * 2 / 5).clamp(56, 80)).unwrap_or(80);
            let [left, viewer] =
                Layout::horizontal([Constraint::Length(left_width), Constraint::Fill(1)])
                    .areas(body);
            let [context, lists] =
                Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).areas(left);
            let list_areas = Layout::vertical([Constraint::Fill(1); 4]).split(lists);
            r.context = Some(context);
            r.lists = ResourceKind::ALL
                .into_iter()
                .zip(list_areas.iter().copied())
                .collect();
            r.viewer = viewer;
        }
        LayoutMode::Narrow => {
            let [tabs, rest] =
                Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).areas(body);
            // Border (2) + header row (1) + data rows, at most 40% of the remaining height.
            let wanted = u16::try_from(list_rows.saturating_add(3)).unwrap_or(u16::MAX);
            let list_height = wanted.min(rest.height * 2 / 5).max(4.min(rest.height));
            let [list, viewer] =
                Layout::vertical([Constraint::Length(list_height), Constraint::Fill(1)])
                    .areas(rest);
            r.list_tabs = Some(tabs);
            r.lists = vec![(visible_list, list)];
            r.viewer = viewer;
        }
        LayoutMode::TooSmall => unreachable!("handled above"),
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_regions_tile_the_screen() {
        let area = Rect::new(0, 0, 160, 48);
        let r = regions(area, LayoutMode::Wide, ResourceKind::Pods, 6, false);
        assert_eq!(r.lists.len(), 4);
        assert_eq!(r.context.unwrap().height, 3);
        assert_eq!(r.lists[0].1.width, 64);
        assert_eq!(r.viewer.x, 64);
        assert_eq!(r.viewer.width, 96);
        assert_eq!(r.footer.y, 47);
    }

    #[test]
    fn narrow_list_height_is_capped() {
        let area = Rect::new(0, 0, 92, 48);
        let small = regions(area, LayoutMode::Narrow, ResourceKind::Services, 5, false);
        assert_eq!(small.lists[0].1.height, 8);
        let big = regions(area, LayoutMode::Narrow, ResourceKind::Pods, 500, false);
        assert_eq!(big.lists[0].1.height, 43 * 2 / 5);
    }

    #[test]
    fn huge_terminal_does_not_overflow() {
        let area = Rect::new(0, 0, u16::MAX, 100);
        let r = regions(area, LayoutMode::Wide, ResourceKind::Pods, 6, false);
        assert_eq!(r.lists[0].1.width, 80);
    }

    #[test]
    fn zoom_gives_viewer_the_body() {
        let area = Rect::new(0, 0, 160, 48);
        let r = regions(area, LayoutMode::Wide, ResourceKind::Pods, 6, true);
        assert!(r.lists.is_empty() && r.context.is_none());
        assert_eq!(r.viewer, Rect::new(0, 1, 160, 46));
    }
}
