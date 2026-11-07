//! Help overlay listing every key binding.

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use super::theme::Theme;

pub const BINDINGS: &[(&str, &str)] = &[
    ("↑↓ jk PgUp/Dn", "move or scroll (pages of 10)"),
    ("enter", "show the row in the viewer"),
    ("tab / shift+tab", "cycle focus (viewer: ↑↓ enter jump)"),
    ("1-4", "jump to list"),
    ("←→", "viewer tabs"),
    ("/", "filter the list / search the logs"),
    ("x / n", "switch context / namespace"),
    ("r", "refresh now"),
    ("c f p J w", "container, follow, prev, JSON, wrap"),
    ("z", "zoom viewer"),
    ("? / esc", "help / close or leave zoom"),
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
