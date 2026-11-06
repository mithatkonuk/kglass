//! Resource list panes, the context box and the narrow layout's list tabs.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Paragraph, Row, Table, TableState, Wrap};
use unicode_width::UnicodeWidthStr;

use super::layout::LayoutMode;
use super::theme::Theme;
use crate::app::{App, Focus, PaneStatus};
use crate::cluster::fetch::{FetchError, MAX_ITEMS};
use crate::model::{PodStatus, ResourceKind, format_age, format_ports};
use crate::text::truncate;

const COLUMN_SPACING: u16 = 1;

/// One table cell: optional leading symbol (not truncated) and text.
struct CellText {
    symbol: Option<(&'static str, Style)>,
    text: String,
    style: Style,
}

impl CellText {
    fn new(text: impl Into<String>, style: Style) -> Self {
        CellText {
            symbol: None,
            text: text.into(),
            style,
        }
    }

    fn with_symbol(symbol: &'static str, symbol_style: Style, text: impl Into<String>) -> Self {
        CellText {
            symbol: Some((symbol, symbol_style)),
            text: text.into(),
            style: Style::new(),
        }
    }

    fn render(self, width: u16) -> Cell<'static> {
        let width = usize::from(width);
        match self.symbol {
            Some((sym, sym_style)) => {
                let room = width.saturating_sub(sym.width() + 1);
                Cell::from(Line::from(vec![
                    Span::styled(sym, sym_style),
                    Span::raw(" "),
                    Span::styled(truncate(&self.text, room), self.style),
                ]))
            }
            None => Cell::from(Span::styled(truncate(&self.text, width), self.style)),
        }
    }
}

fn pod_status_style(status: PodStatus, theme: &Theme) -> Style {
    match status {
        PodStatus::Running => theme.ok,
        PodStatus::Pending | PodStatus::Terminating => theme.warn,
        PodStatus::Completed => theme.muted,
        PodStatus::Failed | PodStatus::CrashLoopBackOff | PodStatus::Failing => theme.err,
    }
}

fn storage_status_style(status: &str, theme: &Theme) -> Style {
    match status {
        "Bound" => theme.ok,
        "Pending" | "Released" => theme.warn,
        "Failed" | "Lost" => theme.err,
        _ => Style::new(),
    }
}

fn columns(kind: ResourceKind, mode: LayoutMode) -> Vec<(&'static str, Constraint)> {
    use Constraint::{Fill, Length};
    match kind {
        ResourceKind::Pods => vec![
            ("NAME", Fill(1)),
            ("STATUS", Length(16)),
            ("READY", Length(5)),
            ("RST", Length(3)),
            ("AGE", Length(6)),
        ],
        ResourceKind::Images => vec![
            ("REPOSITORY", Fill(1)),
            ("TAG / DIGEST", Length(18)),
            ("USED", Length(4)),
        ],
        ResourceKind::Services if mode == LayoutMode::Wide => vec![
            ("NAME", Fill(1)),
            ("TYPE", Length(12)),
            ("CLUSTER-IP", Length(12)),
            ("PORTS", Length(14)),
            ("EP", Length(3)),
        ],
        ResourceKind::Services => vec![
            ("NAME", Fill(1)),
            ("TYPE", Length(12)),
            ("PORTS", Length(14)),
            ("EP", Length(3)),
        ],
        ResourceKind::Storage => vec![
            ("KIND", Length(4)),
            ("NAME", Fill(1)),
            ("STATUS", Length(8)),
            ("CAP", Length(5)),
            ("MODE", Length(4)),
            ("CLASS", Length(5)),
        ],
    }
}

fn rows(app: &App, kind: ResourceKind, mode: LayoutMode, theme: &Theme) -> Vec<Vec<CellText>> {
    match kind {
        ResourceKind::Pods => app
            .snapshot
            .pods
            .iter()
            .map(|p| {
                let style = pod_status_style(p.status, theme);
                let (ready, total) = p.ready();
                let restarts = p.restarts();
                let rst_style = match restarts {
                    0 => Style::new(),
                    1..=9 => theme.warn,
                    _ => theme.err,
                };
                vec![
                    CellText::with_symbol(p.status.symbol(), style, p.name.as_str()),
                    CellText::new(p.status_label(), style),
                    CellText::new(format!("{ready}/{total}"), Style::new()),
                    CellText::new(restarts.to_string(), rst_style),
                    CellText::new(format_age(p.age), theme.muted),
                ]
            })
            .collect(),
        ResourceKind::Images => app
            .images
            .iter()
            .map(|i| {
                let tag = i.image.tag_or_digest();
                let tag_style = if tag.starts_with('@') {
                    theme.derived
                } else {
                    Style::new()
                };
                vec![
                    CellText::new(i.image.repository.as_str(), Style::new()),
                    CellText::new(tag, tag_style),
                    CellText::new(i.users.len().to_string(), Style::new()),
                ]
            })
            .collect(),
        ResourceKind::Services => app
            .snapshot
            .services
            .iter()
            .map(|s| {
                let ep_style = if s.ready_endpoints == 0 {
                    theme.warn
                } else {
                    theme.ok
                };
                let mut cells = vec![
                    CellText::new(s.name.as_str(), Style::new()),
                    CellText::new(s.service_type.label(), theme.muted),
                ];
                if mode == LayoutMode::Wide {
                    cells.push(CellText::new(s.cluster_ip.as_str(), Style::new()));
                }
                cells.push(CellText::new(format_ports(&s.ports), Style::new()));
                cells.push(CellText::new(s.ready_endpoints.to_string(), ep_style));
                cells
            })
            .collect(),
        ResourceKind::Storage => app
            .storage_rows()
            .map(|s| {
                vec![
                    CellText::new(s.kind.label(), theme.muted),
                    CellText::new(s.name.as_str(), Style::new()),
                    CellText::new(
                        s.status.as_str(),
                        storage_status_style(s.status.as_str(), theme),
                    ),
                    CellText::new(s.capacity.as_str(), Style::new()),
                    CellText::new(s.access_modes.as_str(), Style::new()),
                    CellText::new(s.storage_class.as_str(), Style::new()),
                ]
            })
            .collect(),
    }
}

fn pane_title(app: &App, kind: ResourceKind) -> String {
    let filter = app.filter(kind);
    let mut title = if filter.is_empty() {
        format!(
            "[{}]─{} {}",
            kind.number(),
            kind.title(),
            app.row_count(kind)
        )
    } else {
        format!(
            "[{}]─{} {}/{} · filter: {filter}",
            kind.number(),
            kind.title(),
            app.row_count(kind),
            app.total_rows(kind)
        )
    };
    if app.is_truncated(kind) {
        title.push_str(&format!(" · first {MAX_ITEMS}"));
    }
    title
}

fn pane_subtitle(app: &App, kind: ResourceKind) -> String {
    match kind {
        ResourceKind::Pods if app.all_namespaces => "all namespaces".to_string(),
        ResourceKind::Pods => format!("ns {}", app.snapshot.namespace),
        ResourceKind::Images => "derived from pod specs".to_string(),
        ResourceKind::Services => String::new(),
        ResourceKind::Storage => "PVCs + PVs".to_string(),
    }
}

pub fn draw_list(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    kind: ResourceKind,
    mode: LayoutMode,
    theme: &Theme,
) {
    let focused = app.focus == Focus::List(kind);
    let border = if focused {
        theme.border_focused
    } else {
        theme.border
    };
    let mut title = vec![Span::styled(pane_title(app, kind), theme.title)];
    if app.is_stale(kind) {
        title.push(Span::styled(" · STALE", theme.warn));
    }
    let block = Block::bordered()
        .border_style(border)
        .title(Line::from(title))
        .title_top(Line::styled(pane_subtitle(app, kind), theme.muted).right_aligned());
    let mut inner = block.inner(area);
    frame.render_widget(block, area);
    app.hits.panes.push((area, kind));

    let status = app.panes[kind.index()].clone();
    if app.row_count(kind) == 0 {
        let lines = match &status {
            PaneStatus::Loading { since_tick } => vec![loading_line(app, kind, *since_tick, theme)],
            PaneStatus::Failed(e) => error_lines(e, theme),
            PaneStatus::Ready | PaneStatus::Partial(_) => vec![Line::styled(
                if app.all_namespaces {
                    format!("No {} in any namespace", kind.title().to_lowercase())
                } else {
                    format!(
                        "No {} in {}",
                        kind.title().to_lowercase(),
                        app.snapshot.namespace
                    )
                },
                theme.muted,
            )],
        };
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        return;
    }

    // Rows are shown; a failed or partial refresh gets a one-line note at the bottom.
    let note = match &status {
        PaneStatus::Partial(e) => Some(format!("PVs hidden: {e}")),
        PaneStatus::Failed(e) => Some(format!("refresh failed: {e}")),
        PaneStatus::Loading { .. } | PaneStatus::Ready => None,
    };
    if let Some(note) = note
        && inner.height > 2
    {
        let [rows, note_area] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(inner);
        frame.render_widget(
            Paragraph::new(Span::styled(
                truncate(&note, usize::from(note_area.width)),
                theme.warn,
            )),
            note_area,
        );
        inner = rows;
    }

    let cols = columns(kind, mode);
    let constraints: Vec<Constraint> = std::iter::once(Constraint::Length(1))
        .chain(cols.iter().map(|(_, c)| *c))
        .collect();
    let widths: Vec<u16> = Layout::horizontal(constraints)
        .spacing(COLUMN_SPACING)
        .split(Rect::new(0, 0, inner.width, 1))
        .iter()
        .map(|r| r.width)
        .collect();

    let header = Row::new(
        std::iter::once(Cell::from("")).chain(cols.iter().zip(&widths[1..]).map(|((h, _), w)| {
            Cell::from(Span::styled(truncate(h, usize::from(*w)), theme.muted))
        })),
    );
    // Rows, visible indices and their resources are computed once per frame.
    let mut all_rows: Vec<Option<Vec<CellText>>> =
        rows(app, kind, mode, theme).into_iter().map(Some).collect();
    let refs = app.visible_refs(kind);
    let table_rows: Vec<Row> = app
        .visible_rows(kind)
        .into_iter()
        .filter_map(|i| all_rows.get_mut(i).and_then(Option::take))
        .zip(&refs)
        .map(|(cells, r)| {
            let viewed = app.selected.as_ref() == Some(r);
            let marker = Cell::from(Span::styled(if viewed { "›" } else { " " }, theme.key));
            Row::new(
                std::iter::once(marker).chain(
                    cells
                        .into_iter()
                        .zip(&widths[1..])
                        .map(|(c, w)| c.render(*w)),
                ),
            )
        })
        .collect();

    let body_rows = usize::from(inner.height.saturating_sub(1));
    let cursor = &mut app.cursors[kind.index()];
    cursor.scroll_into_view(body_rows);
    let (offset, selected, len) = (cursor.offset(), cursor.selected(), cursor.len());
    let mut state = TableState::new()
        .with_offset(offset)
        .with_selected(selected);
    let highlight = if focused {
        theme.cursor_focused
    } else {
        theme.cursor_unfocused
    };
    let table = Table::new(table_rows, widths.iter().map(|w| Constraint::Length(*w)))
        .header(header)
        .column_spacing(COLUMN_SPACING)
        .row_highlight_style(highlight);
    frame.render_stateful_widget(table, inner, &mut state);

    let visible = body_rows.min(len.saturating_sub(offset));
    for (n, idx) in (offset..offset + visible).enumerate() {
        let y = inner.y + 1 + u16::try_from(n).unwrap_or(u16::MAX);
        app.hits
            .rows
            .push((Rect::new(inner.x, y, inner.width, 1), kind, idx));
    }
}

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn loading_line(app: &App, kind: ResourceKind, since_tick: u64, theme: &Theme) -> Line<'static> {
    let elapsed = app.ticks().saturating_sub(since_tick);
    let frame = SPINNER[usize::try_from(elapsed % 10).unwrap_or(0)];
    let secs = elapsed * crate::runtime::TICK.as_millis() as u64 / 1000;
    Line::styled(
        format!("{frame} loading {}… ({secs}s)", kind.title().to_lowercase()),
        theme.key,
    )
}

/// Explain a failed list, with the exact permission needed for 403s.
fn error_lines(error: &FetchError, theme: &Theme) -> Vec<Line<'static>> {
    match error {
        FetchError::Forbidden {
            verb,
            resource,
            scope,
            ..
        } => vec![
            Line::styled("✕ Forbidden (403)", theme.err),
            Line::raw(format!("cannot {verb} {resource} at the {scope} scope")),
            Line::default(),
            Line::styled(
                format!("Needed (read-only): {resource}: get, list"),
                theme.muted,
            ),
        ],
        // The viewer explains authentication problems once (see viewer::auth_help).
        FetchError::Unauthorized(_) => {
            vec![Line::styled("✕ Unauthorized (401) — see viewer", theme.err)]
        }
        other => vec![
            Line::styled(format!("✕ {other}"), theme.err),
            Line::styled("r retry", theme.muted),
        ],
    }
}

pub fn draw_context(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let snap = &app.snapshot;
    let mut spans = vec![
        Span::styled("context ", theme.muted),
        Span::styled(snap.context.as_str(), theme.title),
        Span::styled("  namespace ", theme.muted),
        Span::styled(snap.namespace.as_str(), theme.title),
    ];
    if !snap.user.as_str().is_empty() {
        spans.push(Span::styled("  user ", theme.muted));
        spans.push(Span::styled(snap.user.as_str(), theme.title));
    }
    let line = Line::from(spans);
    frame.render_widget(
        Paragraph::new(line).block(
            Block::bordered()
                .border_style(theme.border)
                .title(Span::styled("[0]─Context", theme.title)),
        ),
        area,
    );
}

/// Narrow layout: `[1] Pods 6  [2] Images 7 …` with the shown list highlighted.
pub fn draw_tabs(frame: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    let block = Block::bordered().border_style(theme.border);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let shown = app.visible_list();
    let mut spans = Vec::new();
    let mut x = inner.x;
    for kind in ResourceKind::ALL {
        let label = format!(
            "[{}] {} {}",
            kind.number(),
            kind.title(),
            app.row_count(kind)
        );
        let width = u16::try_from(label.width()).unwrap_or(u16::MAX);
        let style = if kind == shown {
            theme.accent
        } else {
            theme.muted
        };
        app.hits
            .list_tabs
            .push((Rect::new(x, inner.y, width, 1).intersection(inner), kind));
        spans.push(Span::styled(label, style));
        spans.push(Span::raw("  "));
        x = x.saturating_add(width + 2);
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), inner);
}
