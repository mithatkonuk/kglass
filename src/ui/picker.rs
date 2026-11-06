//! Context / namespace picker popup.

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use super::theme::Theme;
use crate::state::{Picker, PickerKind};
use crate::text::truncate;

/// Widest the popup gets.
const MAX_WIDTH: u16 = 80;
/// Rows used by the filter line, the gap below it and the status line.
const CHROME_ROWS: u16 = 3;

pub fn draw(frame: &mut Frame, area: Rect, picker: &Picker, theme: &Theme) {
    let matches = picker.matches();
    let wanted = u16::try_from(matches.len())
        .unwrap_or(u16::MAX)
        .saturating_add(CHROME_ROWS + 2)
        .max(CHROME_ROWS + 4);
    let height = wanted.min(area.height.saturating_sub(2)).max(1);
    let width = area.width.saturating_sub(4).min(MAX_WIDTH);
    let [row] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    let [popup] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(row);

    let (title, noun) = match picker.kind {
        PickerKind::Context => ("[x]─Switch context", "contexts · from kubeconfig"),
        PickerKind::Namespace => ("[n]─Switch namespace", "namespaces"),
    };
    let inner_width = usize::from(width.saturating_sub(2));
    let list_rows = usize::from(height.saturating_sub(2 + CHROME_ROWS));

    let mut lines = vec![
        Line::from(vec![
            Span::styled("filter › ", theme.muted),
            Span::styled(format!("{}█", picker.query), theme.accent),
        ]),
        Line::default(),
    ];
    if picker.loading {
        lines.push(Line::styled("loading namespaces…", theme.key));
    } else if let Some(error) = &picker.error {
        lines.push(Line::styled(
            truncate(&format!("✕ cannot list namespaces: {error}"), inner_width),
            theme.err,
        ));
        lines.push(Line::styled(
            "type a namespace name and press enter",
            theme.muted,
        ));
    } else if picker.invalid_name() {
        lines.push(Line::styled(
            "not a valid namespace name (lowercase letters, digits and '-')",
            theme.warn,
        ));
    } else {
        let cursor = picker.cursor();
        let offset = (cursor + 1).saturating_sub(list_rows);
        let name_width = matches
            .iter()
            .map(|i| i.name.as_str().chars().count())
            .max()
            .unwrap_or(0)
            .min(inner_width / 2);
        for (i, item) in matches.iter().enumerate().skip(offset).take(list_rows) {
            let marker = if item.current { "●" } else { "○" };
            let name = truncate(item.name.as_str(), name_width);
            let detail_room = inner_width.saturating_sub(name_width + 5);
            let line = Line::from(vec![
                Span::raw(format!("{marker} {name:<name_width$}   ")),
                Span::styled(truncate(item.detail.as_str(), detail_room), theme.muted),
            ]);
            lines.push(if i == cursor {
                line.style(theme.cursor_focused)
            } else {
                line
            });
        }
    }
    let status = format!(
        "{} of {} {noun} · enter switch · esc cancel",
        matches.len(),
        picker.total()
    );

    let block = Block::bordered()
        .title(Span::styled(title, theme.title))
        .title_bottom(Line::styled(truncate(&status, inner_width), theme.muted))
        .border_style(theme.border_focused);
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}
