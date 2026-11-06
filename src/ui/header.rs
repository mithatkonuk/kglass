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

    let mut spans = vec![
        Span::styled(name, theme.accent.patch(theme.title)),
        Span::raw("  "),
        Span::styled("ctx ", theme.muted),
        Span::styled(snap.context.as_str(), theme.title),
        Span::raw(if wide { " ▾  " } else { "  " }),
        Span::styled("ns ", theme.muted),
        Span::styled(ns, theme.title),
        Span::raw(if wide { " ▾  " } else { "  " }),
        Span::styled("● Connected", theme.ok),
    ];
    if wide {
        spans.extend([
            Span::raw("  "),
            Span::styled(
                format!("api {} · {}", snap.api_version, snap.distro),
                theme.muted,
            ),
            Span::raw("  "),
            Span::styled("⟳ sample data · every 10s", theme.muted),
        ]);
    }

    let badge_width = u16::try_from(badge.chars().count()).unwrap_or(0);
    let [left, right] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(badge_width)]).areas(area);
    frame.render_widget(Paragraph::new(Line::from(spans)), left);
    frame.render_widget(
        Paragraph::new(Span::styled(badge, theme.key.patch(theme.cursor_unfocused))),
        right,
    );
}
