//! Header line: name, context, namespace, connection and mode badge.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::layout::LayoutMode;
use super::theme::Theme;
use crate::app::App;

pub fn draw(frame: &mut Frame, area: Rect, app: &App, mode: LayoutMode, theme: &Theme) {
    let snap = &app.snapshot;
    let ns = if app.all_namespaces {
        "all"
    } else {
        snap.namespace.as_str()
    };
    let wide = mode == LayoutMode::Wide;
    let (name, badge) = if wide {
        ("Kubeglass", " READ-ONLY ")
    } else {
        ("KG", " RO ")
    };

    let spans = vec![
        Span::styled(name, theme.accent.patch(theme.title)),
        Span::raw("  "),
        Span::styled("ctx ", theme.muted),
        Span::styled(snap.context.as_str(), theme.title),
        Span::raw("  "),
        Span::styled("ns ", theme.muted),
        Span::styled(ns, theme.title),
        Span::raw("  "),
        // Phase 1 never talks to a cluster: say so instead of showing a connection state.
        Span::styled(
            if wide {
                "◌ SAMPLE DATA · not connected to a cluster"
            } else {
                "◌ SAMPLE DATA"
            },
            theme.warn,
        ),
    ];

    let badge_width = u16::try_from(badge.chars().count()).unwrap_or(0);
    let [left, right] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(badge_width)]).areas(area);
    frame.render_widget(Paragraph::new(Line::from(spans)), left);
    frame.render_widget(
        Paragraph::new(Span::styled(badge, theme.key.patch(theme.cursor_unfocused))),
        right,
    );
}
