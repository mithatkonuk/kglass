//! The main viewer pane: tab strip, title and per-resource content.

mod image;
mod pod;
mod service;
mod storage;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use super::theme::Theme;
use crate::app::{App, Focus, ViewerTab, YamlState};
use crate::fake::sample_yaml;
use crate::model::{EventType, ResourceRef, format_age};
use crate::text::{SafeText, truncate};

const TAB_SEPARATOR: &str = " - ";
/// Width of the value column's label in key/value sections.
const LABEL_WIDTH: usize = 16;

pub fn draw(frame: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    let focused = app.focus == Focus::Viewer;
    let block = Block::bordered()
        .border_style(if focused {
            theme.border_focused
        } else {
            theme.border
        })
        .title_top(tab_strip(area, app, theme))
        .title_top(title(area, app, theme).right_aligned())
        .title_bottom(Line::styled(hint(app), theme.muted));
    let mut inner = block.inner(area);
    frame.render_widget(block, area);
    app.hits.viewer = area;

    // A deleted selection keeps its last content below an explanation.
    if let Some(notice) = app.viewer_notice.clone()
        && inner.height > 3
    {
        let [banner, rest] =
            Layout::vertical([Constraint::Length(2), Constraint::Fill(1)]).areas(inner);
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(format!("⊘ {notice}"), theme.warn),
                Line::styled(
                    "  Showing last known data; the live stream is closed.",
                    theme.muted,
                ),
            ]),
            banner,
        );
        inner = rest;
    }

    match app.current_tab() {
        None => {
            app.viewer_rows = usize::from(inner.height);
            frame.render_widget(
                Paragraph::new(Span::styled("Nothing selected", theme.muted)),
                inner,
            );
        }
        Some(ViewerTab::Logs) => pod::draw_logs(frame, inner, app, theme),
        Some(tab) => {
            app.viewer_rows = usize::from(inner.height);
            let lines = content(app, tab, theme);
            let max_scroll = u16::try_from(lines.len().saturating_sub(1)).unwrap_or(u16::MAX);
            app.viewer_scroll = app.viewer_scroll.min(max_scroll);
            frame.render_widget(
                Paragraph::new(lines)
                    .wrap(Wrap { trim: false })
                    .scroll((app.viewer_scroll, 0)),
                inner,
            );
        }
    }
}

/// `Logs - Summary - …` with the active tab highlighted; records click targets.
fn tab_strip(area: Rect, app: &mut App, theme: &Theme) -> Line<'static> {
    let mut spans = Vec::new();
    let mut x = area.x.saturating_add(1);
    for (i, tab) in app.current_tabs().iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(TAB_SEPARATOR, theme.muted));
            x = x.saturating_add(TAB_SEPARATOR.len() as u16);
        }
        let name = tab.title();
        let width = u16::try_from(name.width()).unwrap_or(u16::MAX);
        let style = if i == app.tab {
            theme.accent
        } else {
            theme.muted
        };
        spans.push(Span::styled(name, style));
        app.hits
            .tabs
            .push((Rect::new(x, area.y, width, 1).intersection(area), i));
        x = x.saturating_add(width);
    }
    Line::from(spans)
}

fn title(area: Rect, app: &App, theme: &Theme) -> Line<'static> {
    let Some(r) = &app.selected else {
        return Line::default();
    };
    let tabs_width: usize = app
        .current_tabs()
        .iter()
        .map(|t| t.title().width() + TAB_SEPARATOR.len())
        .sum();
    // Corners (2), a gap after the tabs (2), the kind label and its space.
    let room = usize::from(area.width).saturating_sub(tabs_width + r.kind_label().len() + 5);
    Line::from(vec![
        Span::styled(format!("{} ", r.kind_label()), theme.muted),
        Span::styled(truncate(r.name().as_str(), room.max(8)), theme.title),
    ])
}

fn hint(app: &App) -> String {
    if !app.related().is_empty() {
        return " ↑↓ choose · enter jump · ←→ tabs · z zoom ".to_string();
    }
    match app.current_tab() {
        Some(ViewerTab::Logs) => " ←→ tabs · c container · f follow · z zoom ".to_string(),
        Some(_) => " ←→ tabs · ↑↓ scroll · z zoom ".to_string(),
        None => String::new(),
    }
}

fn content(app: &App, tab: ViewerTab, theme: &Theme) -> Vec<Line<'static>> {
    let Some(r) = app.selected.clone() else {
        return Vec::new();
    };
    match tab {
        ViewerTab::Events => events_lines(app, &r, theme),
        ViewerTab::Yaml => yaml_lines(app, &r, theme),
        _ => match &r {
            ResourceRef::Pod(_) => pod::lines(app, tab, theme),
            ResourceRef::Image(_) => image::lines(app, tab, theme),
            ResourceRef::Service(_) => service::lines(app, tab, theme),
            ResourceRef::Pvc(_) | ResourceRef::Pv(_) => storage::lines(app, tab, theme),
        },
    }
}

fn yaml_lines(app: &App, r: &ResourceRef, theme: &Theme) -> Vec<Line<'static>> {
    let text = if app.is_live() {
        match app.yaml_state() {
            Some(YamlState::Ready(text)) => text.clone(),
            Some(YamlState::Failed(e)) => return vec![Line::styled(format!("✕ {e}"), theme.err)],
            Some(YamlState::Loading) | None => {
                return vec![Line::styled("loading YAML…", theme.muted)];
            }
        }
    } else {
        sample_yaml(r, &app.snapshot)
    };
    // YAML comes from the cluster: sanitize every line before display.
    text.lines()
        .map(|l| Line::raw(SafeText::from(l).to_string()))
        .collect()
}

/// Which related item is highlighted: only while the viewer has focus.
pub(crate) fn related_cursor(app: &App) -> Option<usize> {
    (app.focus == Focus::Viewer).then_some(app.related_cursor)
}

/// Highlight `line` when it is related item `index` under the cursor.
pub(crate) fn mark(
    line: Line<'static>,
    index: usize,
    cursor: Option<usize>,
    theme: &Theme,
) -> Line<'static> {
    if cursor == Some(index) {
        line.style(theme.cursor_focused)
    } else {
        line
    }
}

/// A "Jump to" list of the related resources, for tabs that do not list them inline.
pub(crate) fn jump_section(app: &App, theme: &Theme) -> Vec<Line<'static>> {
    let related = app.related();
    if related.is_empty() {
        return Vec::new();
    }
    let cursor = related_cursor(app);
    let mut lines = vec![Line::default(), section("Jump to", theme)];
    for (i, r) in related.iter().enumerate() {
        let line = Line::from(vec![
            Span::styled("  → ", theme.key),
            Span::raw(format!("{} {}  ", r.kind_label(), r.name())),
            derived_tag(theme),
        ]);
        lines.push(mark(line, i, cursor, theme));
    }
    lines
}

pub(crate) fn section(title: &str, theme: &Theme) -> Line<'static> {
    Line::styled(title.to_string(), theme.title)
}

pub(crate) fn kv(label: &str, value: impl Into<String>, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {label:<LABEL_WIDTH$}"), theme.muted),
        Span::raw(value.into()),
    ])
}

pub(crate) fn derived_tag(theme: &Theme) -> Span<'static> {
    Span::styled("[derived]", theme.derived)
}

pub(crate) fn note(text: &str, theme: &Theme) -> Line<'static> {
    Line::styled(format!("  {text}"), theme.muted)
}

/// Events for `r`, newest first, as an aligned table.
pub(crate) fn events_lines(app: &App, r: &ResourceRef, theme: &Theme) -> Vec<Line<'static>> {
    let mut events: Vec<_> = app
        .snapshot
        .events
        .iter()
        .filter(|e| &e.object == r)
        .collect();
    if events.is_empty() {
        return vec![note("No events for this object", theme)];
    }
    events.sort_by_key(|e| e.age);
    let mut lines = vec![Line::styled(
        format!(
            "{:<8} {:<20} {:>6} {:>5}  MESSAGE",
            "TYPE", "REASON", "AGE", "COUNT"
        ),
        theme.muted,
    )];
    for e in events {
        let (label, style) = match e.event_type {
            EventType::Normal => ("Normal", Style::new()),
            EventType::Warning => ("Warning", theme.warn),
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{label:<8} "), style),
            Span::raw(format!(
                "{:<20} {:>6} {:>5}  {}",
                truncate(e.reason.as_str(), 20),
                format_age(e.age),
                format!("×{}", e.count),
                e.message
            )),
        ]));
    }
    lines
}
