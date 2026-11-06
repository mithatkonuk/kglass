//! Image viewer: reference details and the containers using it.

use ratatui::text::{Line, Span};

use super::{derived_tag, kv, note, section};
use crate::app::{App, ViewerTab};
use crate::model::{ImageUsage, ResourceRef};
use crate::ui::theme::Theme;

fn selected_image(app: &App) -> Option<&ImageUsage> {
    match &app.selected {
        Some(ResourceRef::Image(raw)) => app.images.iter().find(|i| &i.raw == raw),
        _ => None,
    }
}

pub(super) fn lines(app: &App, tab: ViewerTab, theme: &Theme) -> Vec<Line<'static>> {
    let Some(img) = selected_image(app) else {
        return Vec::new();
    };
    match tab {
        ViewerTab::Details => {
            let mut lines = vec![
                section("Image", theme),
                kv("Reference", img.raw.to_string(), theme),
                kv("Repository", img.image.repository.to_string(), theme),
            ];
            if let Some(tag) = &img.image.tag {
                lines.push(kv("Tag", tag.to_string(), theme));
            }
            if let Some(digest) = &img.image.digest {
                lines.push(kv("Digest", digest.to_string(), theme));
            }
            if img.image.tag.is_none() && img.image.digest.is_none() {
                lines.push(kv("Tag", "latest (implicit)", theme));
            }
            lines.push(kv(
                "Used by",
                format!("{} container(s)", img.users.len()),
                theme,
            ));
            lines.push(Line::default());
            lines.push(note(
                "Registry metadata: not retrieved (no registry calls in this version)",
                theme,
            ));
            lines
        }
        ViewerTab::Workloads => {
            let mut lines = vec![Line::from(vec![
                Span::styled("Containers using this image ", theme.title),
                derived_tag(theme),
            ])];
            lines.extend(
                img.users
                    .iter()
                    .map(|(pod, container)| Line::raw(format!("  {pod} → {container}"))),
            );
            lines
        }
        _ => Vec::new(),
    }
}
