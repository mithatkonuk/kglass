//! Footer: key hints and version.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::layout::LayoutMode;
use super::theme::Theme;
use crate::app::App;

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

/// Extra keys that only do something with a live cluster.
const LIVE_HINTS: &[(&str, &str)] = &[("x", "context"), ("n", "namespace"), ("r", "refresh")];

const NARROW_HINTS: &[(&str, &str)] = &[
    ("1-4", "list"),
    ("↑↓", "move"),
    ("tab", "focus"),
    ("z", "zoom"),
    ("?", "help"),
    ("q", "quit"),
];

pub fn draw(frame: &mut Frame, area: Rect, app: &App, mode: LayoutMode, theme: &Theme) {
    if let Some(kind) = app.filter_editing {
        let line = Line::from(vec![
            Span::styled(
                format!("filter {} › ", app.kind_title(kind).to_lowercase()),
                theme.key,
            ),
            Span::styled(format!("{}█", app.filter(kind)), theme.accent),
            Span::styled("  enter done · esc clear", theme.muted),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }
    if app.log_search_editing {
        let line = Line::from(vec![
            Span::styled("search logs › ", theme.key),
            Span::styled(format!("{}█", app.log_search), theme.accent),
            Span::styled("  enter done · esc clear", theme.muted),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }
    let wide = mode == LayoutMode::Wide;
    let mut hints: Vec<(&str, &str)> = if wide {
        WIDE_HINTS.to_vec()
    } else {
        NARROW_HINTS.to_vec()
    };
    if wide {
        // Insert before the help/quit hints.
        let at = hints.len() - 2;
        let mut extra = vec![("/", "filter")];
        if app.is_live() {
            extra.extend_from_slice(LIVE_HINTS);
        }
        hints.splice(at..at, extra);
    }
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
