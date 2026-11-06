//! Pod viewer: logs, summary and containers.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use super::{events_lines, kv, note, section};
use crate::app::{App, LOG_TAIL, LogState, ViewerTab};
use crate::logfmt::format_line;
use crate::model::{Pod, PodStatus, ResourceRef, format_age};
use crate::ui::theme::Theme;

/// Length of `2026-11-06 12:04:11.000` at the start of each sample log line.
const TIMESTAMP_LEN: usize = 23;

pub(super) fn draw_logs(frame: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    let Some(pod) = app.selected_pod().cloned() else {
        return;
    };
    if !pod.has_started() {
        app.viewer_rows = usize::from(area.height);
        let lines = not_started(app, &pod, theme);
        let max_scroll = u16::try_from(lines.len().saturating_sub(1)).unwrap_or(u16::MAX);
        app.viewer_scroll = app.viewer_scroll.min(max_scroll);
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((app.viewer_scroll, 0)),
            area,
        );
        return;
    }
    let [toolbar, body, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);
    app.viewer_rows = usize::from(body.height);

    frame.render_widget(Paragraph::new(toolbar_line(app, theme)), toolbar);
    let lines: Vec<Line> = if app.log_search.is_empty() {
        app.logs
            .visible(app.viewer_rows)
            .map(|l| log_line(&display_text(app, l), theme))
            .collect()
    } else {
        // Searching: the newest matches, `log_search_back` matches up from the bottom.
        let matches = app.log_matches();
        let start = matches
            .len()
            .saturating_sub(app.viewer_rows + app.log_search_back);
        matches
            .iter()
            .skip(start)
            .take(app.viewer_rows)
            .filter_map(|&i| app.logs.line(i))
            .map(|l| log_line(&display_text(app, l), theme))
            .collect()
    };
    frame.render_widget(Paragraph::new(lines), body);

    let (streaming, state) = if app.is_live() {
        match &app.log_state {
            LogState::Streaming => (true, "streaming".to_string()),
            LogState::Ended(reason) if app.logs_previous && reason.as_str() == "stream closed" => {
                (false, "previous logs (complete)".to_string())
            }
            LogState::Ended(reason) => (false, format!("stream ended: {reason}")),
            LogState::Idle => (false, "not streaming".to_string()),
        }
    } else if pod.status == PodStatus::Running {
        (true, "streaming (sample)".to_string())
    } else {
        (false, "stream ended".to_string())
    };
    let dot = if streaming { "●" } else { "○" };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(dot, if streaming { theme.ok } else { theme.muted }),
            Span::styled(
                format!(
                    " {state} · {} lines buffered (cap {}){}",
                    app.logs.len(),
                    app.logs.cap(),
                    if app.log_search.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " · search \"{}\" · {} matches",
                            app.log_search,
                            app.log_matches().len()
                        )
                    }
                ),
                theme.muted,
            ),
        ])),
        status,
    );
}

fn previous_label(app: &App) -> String {
    if app.logs_previous {
        "previous on (p)".to_string()
    } else if !app.is_live() || app.current_container_restarts().unwrap_or(0) == 0 {
        "previous n/a".to_string()
    } else {
        "previous off (p)".to_string()
    }
}

/// Structured lines are formatted unless raw mode is on; other lines show as received.
fn display_text<'a>(app: &App, line: &'a str) -> std::borrow::Cow<'a, str> {
    if app.raw_logs {
        return line.into();
    }
    format_line(line).map_or(line.into(), Into::into)
}

fn toolbar_line(app: &App, theme: &Theme) -> Line<'static> {
    let mut spans = vec![Span::styled("container ▸ ", theme.muted)];
    for (i, name) in app.current_containers().iter().enumerate() {
        let style = if i == app.container {
            theme.accent
        } else {
            theme.muted
        };
        spans.push(Span::styled(name.as_str().to_string(), style));
        spans.push(Span::raw("  "));
    }
    let (follow, follow_style) = if app.logs.follow() {
        ("● on", theme.ok)
    } else {
        ("○ off", theme.warn)
    };
    spans.extend([
        Span::styled(" follow ", theme.muted),
        Span::styled(follow, follow_style),
        Span::styled(format!("   tail {LOG_TAIL}   "), theme.muted),
        Span::styled(previous_label(app), theme.muted),
        Span::styled(
            if app.raw_logs {
                "   raw JSON (J)"
            } else {
                "   formatted (J)"
            },
            theme.muted,
        ),
        Span::styled(
            app.log_hint
                .as_ref()
                .map_or(String::new(), |h| format!("   {h}")),
            theme.warn,
        ),
    ]);
    Line::from(spans)
}

/// Colour the timestamp and level of a `{ts} {LEVEL} …` line without changing its text.
fn log_line(line: &str, theme: &Theme) -> Line<'static> {
    let Some(rest) = line.get(TIMESTAMP_LEN..) else {
        return Line::raw(line.to_string());
    };
    let ts = &line[..TIMESTAMP_LEN];
    let trimmed = rest.trim_start();
    let gap = &rest[..rest.len() - trimmed.len()];
    let level = trimmed.split_whitespace().next().unwrap_or("");
    let after_level = &trimmed[level.len()..];
    let level_style = match level {
        "ERROR" | "FATAL" => theme.err,
        "WARN" => theme.warn,
        "INFO" => theme.ok,
        "DEBUG" | "TRACE" => theme.muted,
        _ => Style::new(),
    };
    Line::from(vec![
        Span::styled(ts.to_string(), theme.muted),
        Span::raw(gap.to_string()),
        Span::styled(level.to_string(), level_style),
        Span::raw(after_level.to_string()),
    ])
}

fn not_started(app: &App, pod: &Pod, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for c in pod.containers.iter().filter(|c| !c.init) {
        let reason = c
            .waiting_reason
            .as_ref()
            .map_or("waiting".to_string(), |r| r.to_string());
        lines.push(Line::styled(
            format!("◌ Container {} has not started ({reason})", c.name),
            theme.warn,
        ));
    }
    lines.push(note("There are no logs yet. Recent events:", theme));
    lines.push(Line::default());
    lines.extend(events_lines(
        app,
        &ResourceRef::Pod(pod.name.clone()),
        theme,
    ));
    lines
}

pub(super) fn lines(app: &App, tab: ViewerTab, theme: &Theme) -> Vec<Line<'static>> {
    let Some(pod) = app.selected_pod() else {
        return Vec::new();
    };
    match tab {
        ViewerTab::Summary => summary(pod, theme),
        ViewerTab::Containers => containers(pod, theme),
        _ => Vec::new(),
    }
}

fn summary(pod: &Pod, theme: &Theme) -> Vec<Line<'static>> {
    let (ready, total) = pod.ready();
    let labels = pod
        .labels
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(", ");
    vec![
        section("Pod", theme),
        kv("Name", pod.name.to_string(), theme),
        kv("Namespace", pod.namespace.to_string(), theme),
        kv(
            "Status",
            format!("{} {}", pod.status.symbol(), pod.status_label()),
            theme,
        ),
        kv("Ready", format!("{ready}/{total}"), theme),
        kv("Restarts", pod.restarts().to_string(), theme),
        kv("Age", format_age(pod.age), theme),
        kv("Node", pod.node.to_string(), theme),
        kv("Pod IP", pod.pod_ip.to_string(), theme),
        kv(
            "Controlled by",
            pod.owner
                .as_ref()
                .map_or("—".to_string(), |o| o.to_string()),
            theme,
        ),
        kv("Labels", labels, theme),
    ]
}

fn containers(pod: &Pod, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let ordered = pod
        .containers
        .iter()
        .filter(|c| !c.init)
        .chain(pod.containers.iter().filter(|c| c.init));
    for c in ordered {
        let mut head = vec![Span::styled(c.name.to_string(), theme.title)];
        if c.init {
            head.push(Span::styled(" [init]", theme.muted));
        }
        lines.push(Line::from(head));
        lines.push(kv("Image", c.image.to_string(), theme));
        lines.push(kv("Ready", if c.ready { "yes" } else { "no" }, theme));
        lines.push(kv("Restarts", c.restarts.to_string(), theme));
        if let Some(reason) = &c.waiting_reason {
            lines.push(kv("Waiting", reason.to_string(), theme));
        }
        lines.push(Line::default());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn colouring_keeps_log_text_unchanged() {
        let theme = Theme::color();
        for raw in [
            "2026-11-06 12:04:11.000 INFO  http    request completed",
            "2026-11-06 12:04:20.177 ERROR ledger  upstream unavailable",
            "2026-11-06 12:04:11.874 DEBUG db      pool stats",
            "short line",
            "2026-11-06 12:04:11.000",
        ] {
            assert_eq!(text(&log_line(raw, &theme)), raw);
        }
    }
}
