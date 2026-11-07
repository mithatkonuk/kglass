//! PersistentVolumeClaim and PersistentVolume viewers.

use ratatui::text::{Line, Span};

use super::{derived_tag, jump_section, kv, mark, note, related_cursor, section};
use crate::app::{App, ViewerTab};
use crate::model::{ResourceRef, StorageItem, StorageKind};
use crate::ui::theme::Theme;

const METADATA_ONLY: &str = "Kubernetes metadata only — storage contents are not inspected.";
const LOCAL_METADATA_ONLY: &str = "Engine metadata only — volume contents are not inspected.";

fn selected_item(app: &App) -> Option<&StorageItem> {
    let (kind, name) = match &app.selected {
        Some(ResourceRef::Pvc(n)) => (StorageKind::Pvc, n),
        Some(ResourceRef::Pv(n)) => (StorageKind::Pv, n),
        _ => return None,
    };
    app.snapshot.storage.iter().find(|s| {
        let same = if kind == StorageKind::Pvc {
            s.kind.is_claim()
        } else {
            s.kind == kind
        };
        same && &s.name == name
    })
}

fn dash(v: Option<&impl ToString>) -> String {
    v.map_or("—".to_string(), ToString::to_string)
}

pub(super) fn lines(app: &App, tab: ViewerTab, theme: &Theme) -> Vec<Line<'static>> {
    let Some(item) = selected_item(app) else {
        return Vec::new();
    };
    let mut lines = match (item.kind, tab) {
        (_, ViewerTab::Details) => {
            let mut lines = details(item, theme);
            lines.extend(jump_section(app, theme));
            lines
        }
        (StorageKind::Pvc | StorageKind::Volume, ViewerTab::MountedBy) => {
            mounted_by(item, related_cursor(app), theme)
        }
        (StorageKind::Pv, ViewerTab::Claim) => {
            let mut lines = vec![
                section("Claim", theme),
                kv("Bound claim", dash(item.bound_to.as_ref()), theme),
                kv("Phase", item.status.to_string(), theme),
            ];
            lines.extend(jump_section(app, theme));
            lines
        }
        (StorageKind::Pv, ViewerTab::StorageDetails) => vec![
            section("Storage", theme),
            kv("Storage class", item.storage_class.to_string(), theme),
            kv("Capacity", item.capacity.to_string(), theme),
            kv("Access modes", item.access_modes.to_string(), theme),
        ],
        _ => Vec::new(),
    };
    lines.push(Line::default());
    let metadata_note = if item.kind == StorageKind::Volume {
        LOCAL_METADATA_ONLY
    } else {
        METADATA_ONLY
    };
    lines.push(note(metadata_note, theme));
    lines
}

fn details(item: &StorageItem, theme: &Theme) -> Vec<Line<'static>> {
    let (title, bound_label) = match item.kind {
        StorageKind::Pvc => ("PersistentVolumeClaim", "Volume"),
        StorageKind::Pv => ("PersistentVolume", "Claim"),
        StorageKind::Volume => ("Volume (local engine)", "Bound to"),
    };
    let mut lines = vec![
        section(title, theme),
        kv("Name", item.name.to_string(), theme),
        kv("Status", item.status.to_string(), theme),
        kv("Capacity", item.capacity.to_string(), theme),
        kv("Access modes", item.access_modes.to_string(), theme),
        kv("Storage class", item.storage_class.to_string(), theme),
        kv(bound_label, dash(item.bound_to.as_ref()), theme),
    ];
    let warning = match (item.kind, item.status.as_str()) {
        (StorageKind::Pvc, "Pending") => Some("⚠ Claim is Pending — no volume is bound yet."),
        (StorageKind::Pv, "Released") => {
            Some("⚠ Released — claim was deleted; volume not reusable until reclaimed.")
        }
        _ => None,
    };
    if let Some(w) = warning {
        lines.push(Line::default());
        lines.push(Line::styled(format!("  {w}"), theme.warn));
    }
    lines
}

fn mounted_by(item: &StorageItem, cursor: Option<usize>, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(vec![
        Span::styled("Pods mounting this claim ", theme.title),
        derived_tag(theme),
    ])];
    if item.mounted_by.is_empty() {
        lines.push(note("Not mounted by any pod", theme));
    }
    lines.extend(item.mounted_by.iter().enumerate().map(|(i, p)| {
        mark(
            Line::styled(format!("  → {p}"), theme.key),
            i,
            cursor,
            theme,
        )
    }));
    lines
}
