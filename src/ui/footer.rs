//! Footer: key hints and version.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::layout::LayoutMode;
use super::theme::Theme;

const WIDE_HINTS: &[(&str, &str)] = &[
    ("↑↓", "navigate"),
    ("enter", "select"),
    ("tab", "focus"),
    ("1-4", "jump to list"),
    ("←→", "tabs"),
    ("z", "zoom"),
    ("?", "help"),
    ("q", "quit"),
];

const NARROW_HINTS: &[(&str, &str)] = &[
    ("1-4", "list"),
    ("↑↓", "move"),
    ("tab", "focus"),
    ("z", "zoom"),
    ("?", "help"),
    ("q", "quit"),
];

pub fn draw(frame: &mut Frame, area: Rect, mode: LayoutMode, theme: &Theme) {
    let hints = if mode == LayoutMode::Wide {
        WIDE_HINTS
    } else {
        NARROW_HINTS
    };
    let spans: Vec<Span> = hints
        .iter()
        .flat_map(|(k, label)| {
            [
                Span::styled(*k, theme.key),
                Span::raw(format!(" {label}  ")),
            ]
        })
        .collect();
    let version = format!("v{} · ? help", env!("CARGO_PKG_VERSION"));
    let version_width = u16::try_from(version.chars().count()).unwrap_or(0);
    let [left, right] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(version_width)]).areas(area);
    frame.render_widget(Paragraph::new(Line::from(spans)), left);
    if mode == LayoutMode::Wide {
        frame.render_widget(Paragraph::new(Span::styled(version, theme.muted)), right);
    }
}
