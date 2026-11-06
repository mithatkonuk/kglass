//! Service viewer: details with endpoint health and selector-matched pods.

use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

use super::{derived_tag, kv, note, section};
use crate::app::{App, ViewerTab};
use crate::derive::pods_matching;
use crate::model::{Pod, ResourceRef, Service};
use crate::ui::theme::Theme;

fn selected_service(app: &App) -> Option<&Service> {
    match &app.selected {
        Some(ResourceRef::Service(name)) => app.snapshot.services.iter().find(|s| &s.name == name),
        _ => None,
    }
}

fn selector_text(s: &Service) -> String {
    if s.selector.is_empty() {
        return "none (endpoints managed manually)".to_string();
    }
    s.selector
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn ports_text(s: &Service) -> String {
    s.ports
        .iter()
        .map(|p| {
            let node = p
                .node_port
                .map_or(String::new(), |n| format!(" (node {n})"));
            format!("{}/{} → {}{node}", p.port, p.protocol, p.target_port)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn is_ready(p: &Pod) -> bool {
    let (ready, total) = p.ready();
    total > 0 && ready == total
}

pub(super) fn lines(app: &App, tab: ViewerTab, theme: &Theme) -> Vec<Line<'static>> {
    let Some(svc) = selected_service(app) else {
        return Vec::new();
    };
    let matching = pods_matching(&svc.selector, &app.snapshot.pods);
    match tab {
        ViewerTab::Details => details(svc, &matching, theme),
        ViewerTab::Endpoints => endpoints(svc, &matching, theme),
        _ => Vec::new(),
    }
}

fn details(svc: &Service, matching: &[&Pod], theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = vec![
        section("Service", theme),
        kv("Name", svc.name.to_string(), theme),
        kv("Type", svc.service_type.label(), theme),
        kv("Cluster IP", svc.cluster_ip.to_string(), theme),
        kv("Ports", ports_text(svc), theme),
        kv("Selector", selector_text(svc), theme),
        Line::default(),
        section("Endpoints", theme),
    ];
    if svc.ready_endpoints == 0 {
        let ready = matching.iter().filter(|p| is_ready(p)).count();
        let verb = if matching.len() == 1 {
            "pod matches"
        } else {
            "pods match"
        };
        lines.push(Line::from(vec![
            Span::styled("  ⚠ ", theme.warn),
            Span::styled(
                "No ready endpoints.",
                theme.warn.add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Traffic to this Service will fail.", theme.warn),
        ]));
        lines.push(Line::styled(
            format!(
                "    {} {verb} the selector; {ready} are Ready.",
                matching.len()
            ),
            theme.warn,
        ));
    } else {
        lines.push(Line::styled(
            format!("  {} ready endpoints", svc.ready_endpoints),
            theme.ok,
        ));
    }
    lines.push(Line::default());
    lines.push(section("Matching pods", theme));
    if matching.is_empty() {
        lines.push(note("No pods match the selector", theme));
    }
    for p in matching {
        lines.push(Line::from(vec![
            Span::styled("  → ", theme.key),
            Span::styled(format!("{:<24}", p.name.as_str()), theme.key),
            Span::raw(format!("{:<18}", p.status.label())),
            derived_tag(theme),
        ]));
    }
    lines.push(Line::default());
    lines.push(note(
        "Selector matches show configuration, not network reachability.",
        theme,
    ));
    lines
}

fn endpoints(svc: &Service, matching: &[&Pod], theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = vec![Line::styled(
        format!("{} ready endpoints", svc.ready_endpoints),
        if svc.ready_endpoints == 0 {
            theme.warn
        } else {
            theme.ok
        },
    )];
    lines.push(Line::default());
    let target = svc.ports.first().map(|p| p.target_port);
    for p in matching {
        let addr = match target {
            Some(port) => format!("{}:{port}", p.pod_ip),
            None => p.pod_ip.to_string(),
        };
        let ready = if is_ready(p) { "ready" } else { "not ready" };
        lines.push(Line::from(vec![
            Span::raw(format!("  {addr:<22}{:<24}{ready:<11}", p.name.as_str())),
            derived_tag(theme),
        ]));
    }
    lines.push(Line::default());
    lines.push(note(
        "Addresses come from pods matching the selector; EndpointSlices are read in a later version.",
        theme,
    ));
    lines
}
