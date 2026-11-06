//! The main viewer pane.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use super::theme::Theme;
use crate::app::{App, Focus};
use crate::text::truncate;

pub fn draw(frame: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    let focused = app.focus == Focus::Viewer;
    let title = match &app.selected {
        Some(r) => {
            // Keep the title inside the top border: corners (2) + kind label + space.
            let room = usize::from(area.width).saturating_sub(r.kind_label().len() + 3);
            Line::from(vec![
                Span::styled(format!("{} ", r.kind_label()), theme.muted),
                Span::styled(truncate(r.name().as_str(), room), theme.title),
            ])
        }
        None => Line::from("Nothing selected"),
    };
    let block = Block::bordered()
        .border_style(if focused {
            theme.border_focused
        } else {
            theme.border
        })
        .title_top(title.right_aligned());
    let inner = block.inner(area);
    frame.render_widget(Paragraph::new("").block(block), area);
    app.hits.viewer = area;
    app.viewer_rows = usize::from(inner.height);
}
