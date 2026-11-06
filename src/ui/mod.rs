//! Rendering. `draw` turns `App` state into a frame and records click targets.

pub mod footer;
pub mod header;
pub mod help;
pub mod hits;
pub mod layout;
pub mod lists;
pub mod theme;
pub mod viewer;

use ratatui::Frame;
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::App;
use layout::{LayoutMode, MIN_SIZE, layout_mode, regions};
use theme::Theme;

pub fn draw(frame: &mut Frame, app: &mut App) {
    app.hits.clear();
    let area = frame.area();
    let mode = layout_mode(area);
    let theme = Theme::for_plain(app.plain);

    if mode == LayoutMode::TooSmall {
        let msg = format!(
            "Terminal too small — need at least {}×{}",
            MIN_SIZE.0, MIN_SIZE.1
        );
        frame.render_widget(Paragraph::new(msg).wrap(Wrap { trim: true }), area);
        return;
    }

    let shown = app.visible_list();
    let r = regions(area, mode, shown, app.row_count(shown), app.zoom);
    header::draw(frame, r.header, app, mode, &theme);
    footer::draw(frame, r.footer, mode, &theme);
    if let Some(context) = r.context {
        lists::draw_context(frame, context, app, &theme);
    }
    if let Some(tabs) = r.list_tabs {
        lists::draw_tabs(frame, tabs, app, &theme);
    }
    for (kind, rect) in &r.lists {
        lists::draw_list(frame, *rect, app, *kind, mode, &theme);
    }
    viewer::draw(frame, r.viewer, app, &theme);
    if app.help {
        help::draw(frame, area, &theme);
    }
}
