//! Help overlay listing every key binding.

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use super::theme::Theme;

pub const BINDINGS: &[(&str, &str)] = &[
    ("↑↓ / j k", "move cursor or scroll viewer"),
    ("PgUp / PgDn", "move or scroll by 10"),
    ("enter", "show the row in the viewer"),
    ("tab / shift+tab", "cycle focus"),
    ("1-4", "jump to list"),
    ("←→", "viewer tabs"),
    ("c", "next container (logs)"),
    ("f", "toggle follow (logs)"),
    ("z", "zoom viewer"),
    ("esc", "close help / leave zoom"),
    ("?", "toggle help"),
    ("q / ctrl+c", "quit"),
    ("mouse", "click rows/tabs; wheel scrolls"),
];

pub fn draw(frame: &mut Frame, area: Rect, theme: &Theme) {
    let height = u16::try_from(BINDINGS.len()).unwrap_or(u16::MAX) + 2;
    let [row] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    let [popup] = Layout::horizontal([Constraint::Max(62)])
        .flex(Flex::Center)
        .areas(row);
    let lines: Vec<Line> = BINDINGS
        .iter()
        .map(|(k, what)| {
            Line::from(vec![
                Span::styled(format!("{k:>16}  "), theme.key),
                Span::raw(*what),
            ])
        })
        .collect();
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .title(Span::styled(" Help ", theme.title))
                .border_style(theme.border_focused),
        ),
        popup,
    );
}
