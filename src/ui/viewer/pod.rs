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
    // Search matches are computed once per frame and reused for the status line.
    let matches = (!app.log_search.is_empty()).then(|| app.log_matches());
    // Which buffered lines are candidates, and where the view is anchored.
    let width = usize::from(body.width);
    let rows = usize::from(body.height);
    let format = |i: usize| {
        app.logs
            .line(i)
            .map(|l| log_line(&display_text(app, l), theme))
    };
    let lines: Vec<Line> = match &matches {
        // Searching: the newest matches, `log_search_back` matches up from the bottom.
        Some(m) => {
            let end = m.len().saturating_sub(app.log_search_back);
            bottom_anchored(
                m[..end].iter().filter_map(|&i| format(i)),
                width,
                rows,
                app.wrap_logs,
            )
        }
        None if app.logs.follow() => bottom_anchored(
            (0..app.logs.len()).filter_map(format),
            width,
            rows,
            app.wrap_logs,
        ),
        None => {
            let top = app.logs.top(app.viewer_rows);
            top_anchored(
                (top..app.logs.len()).filter_map(format),
                width,
                rows,
                app.wrap_logs,
            )
        }
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
                    " {state} · {} lines buffered (cap {}) · wrap {} (w){}",
                    app.logs.len(),
                    app.logs.cap(),
                    if app.wrap_logs { "on" } else { "off" },
                    match &matches {
                        None => String::new(),
                        Some(m) =>
                            format!(" · search \"{}\" · {} matches", app.log_search, m.len()),
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

/// Prefix of continuation rows of a wrapped line.
const CONTINUATION: &str = "  ";

/// Split a styled line into rows of at most `width` display columns, keeping each
/// span's style. Continuation rows are indented by two spaces.
fn wrap_line(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    if width == 0 || line.width() <= width {
        return vec![line];
    }
    let indent = if width > CONTINUATION.len() * 2 {
        CONTINUATION
    } else {
        ""
    };
    let line_style = line.style;
    let mut rows = Vec::new();
    let mut row: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    for span in line.spans {
        let style = span.style;
        let mut piece = String::new();
        for ch in span.content.chars() {
            let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            if used + w > width && used > indent.len() {
                if !piece.is_empty() {
                    row.push(Span::styled(std::mem::take(&mut piece), style));
                }
                rows.push(Line::from(std::mem::take(&mut row)).style(line_style));
                if !indent.is_empty() {
                    row.push(Span::raw(indent));
                }
                used = indent.len();
            }
            piece.push(ch);
            used += w;
        }
        if !piece.is_empty() {
            row.push(Span::styled(piece, style));
        }
    }
    if !row.is_empty() {
        rows.push(Line::from(row).style(line_style));
    }
    rows
}

/// The last `rows` screen rows of `lines` (the newest at the bottom).
fn bottom_anchored(
    lines: impl DoubleEndedIterator<Item = Line<'static>>,
    width: usize,
    rows: usize,
    wrap: bool,
) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    for line in lines.rev() {
        if out.len() >= rows {
            break;
        }
        let mut wrapped = if wrap {
            wrap_line(line, width)
        } else {
            vec![line]
        };
        wrapped.reverse();
        out.extend(wrapped);
    }
    out.truncate(rows);
    out.reverse();
    out
}

/// Screen rows starting with the first of `lines`, up to `rows` rows.
fn top_anchored(
    lines: impl Iterator<Item = Line<'static>>,
    width: usize,
    rows: usize,
    wrap: bool,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for line in lines {
        if out.len() >= rows {
            break;
        }
        if wrap {
            out.extend(wrap_line(line, width));
        } else {
            out.push(line);
        }
    }
    out.truncate(rows);
    out
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
        ViewerTab::Summary => {
            let mut lines = summary(pod, theme);
            lines.extend(super::jump_section(app, theme));
            lines
        }
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
    fn wrapping_splits_by_display_width_and_keeps_styles() {
        let theme = Theme::color();
        let line = log_line("2026-11-06 12:04:11.000 ERROR http    abcdefghij", &theme);
        let rows = wrap_line(line, 20);
        let texts: Vec<String> = rows.iter().map(text).collect();
        assert_eq!(
            texts,
            [
                "2026-11-06 12:04:11.",
                "  000 ERROR http    ",
                "  abcdefghij"
            ]
        );
        assert!(
            texts
                .iter()
                .all(|t| unicode_width::UnicodeWidthStr::width(t.as_str()) <= 20)
        );
        let error_style = rows[1]
            .spans
            .iter()
            .find(|s| s.content.contains("ERROR"))
            .unwrap()
            .style;
        assert_eq!(error_style, theme.err, "level colour survives the split");
    }

    #[test]
    fn wrapping_handles_wide_characters_and_short_lines() {
        let rows = wrap_line(Line::raw("日本語テキスト"), 6);
        let texts: Vec<String> = rows.iter().map(text).collect();
        assert_eq!(texts, ["日本語", "  テキ", "  スト"]);
        assert_eq!(wrap_line(Line::raw("short"), 80).len(), 1);
        assert_eq!(wrap_line(Line::raw(""), 10).len(), 1);
        assert_eq!(
            wrap_line(Line::raw("abc"), 0).len(),
            1,
            "zero width never loops"
        );
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
