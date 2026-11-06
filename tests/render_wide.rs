mod support;

use kglass::app::{App, AppOptions};
use kglass::fake::sample_snapshot;
use kglass::text::SafeText;
use ratatui::style::Color;
use support::*;
use unicode_width::UnicodeWidthStr;

#[test]
fn wide_matches_pdf_page1_text() {
    let s = render_to_string(&mut app(), 160, 48);
    for needle in [
        "Kubeglass",
        "ctx dev-east",
        "ns payments",
        "READ-ONLY",
        "[0]─Context",
        "user sso-readonly",
        "[1]─Pods 6",
        "[2]─Images 7",
        "[3]─Services 5",
        "[4]─Storage 5",
        "api-7c9d8f-x2k4",
        "CrashLoopBackOff",
        "@sha256:9f2c…e81a",
        "80→8080/TCP",
        "443:31443/TCP",
        "pvc-77b2d4f1-141f",
        "Released",
        "1-4 jump to list",
        "v0.1.0",
    ] {
        assert!(s.contains(needle), "missing {needle:?}\n{s}");
    }
}

#[test]
fn status_has_symbol_and_label() {
    let s = render_to_string(&mut app(), 160, 48);
    for needle in [
        "✕ billing-0",
        "◌ ledger-sync-5f7d-h2c",
        "✓ migrate-28841-zt5",
        "● worker-65b9c6-p2m8",
    ] {
        assert!(s.contains(needle), "missing {needle:?}\n{s}");
    }
}

#[test]
fn viewed_row_has_marker() {
    let s = render_to_string(&mut app(), 160, 48);
    assert!(s.contains("› ● api-7c9d8f-x2k4"), "{s}");
    assert!(
        !s.contains("› ● api-7c9d8f-q8n1"),
        "only the viewed row is marked"
    );
}

#[test]
fn too_small_never_panics() {
    for (w, h) in [(0, 0), (1, 1), (40, 10), (59, 30), (200, 15)] {
        let s = render_to_string(&mut app(), w, h);
        if w >= 40 {
            assert!(s.contains("Terminal too small"), "{w}x{h}\n{s}");
        }
    }
}

#[test]
fn hostile_names_are_clean_and_truncated() {
    let mut snap = sample_snapshot();
    snap.pods[0].name = SafeText::from(
        "\x1b]0;pwn\x07evil-\x1b[31m日本語ポッド-with-a-very-very-long-name-indeed-0123456789",
    );
    let mut a = App::new(snap, AppOptions::default());
    let s = render_to_string(&mut a, 160, 48);
    assert!(!s.contains('\x1b'));
    assert!(!s.contains("pwn"));
    assert!(s.contains("evil-日本語"), "{s}");
    let row = s
        .lines()
        .find(|l| l.contains("› ● evil-"))
        .expect("pod row rendered");
    let pods_pane: String = row.chars().take(64).collect();
    assert!(
        pods_pane.contains('…'),
        "list cell is truncated: {pods_pane}"
    );
    assert!(!pods_pane.contains("indeed"), "{pods_pane}");
    for line in s.lines() {
        assert_eq!(line.width(), 160, "no line overflows: {line}");
    }
}

#[test]
fn empty_list_shows_placeholder() {
    let mut snap = sample_snapshot();
    snap.services.clear();
    let s = render_to_string(&mut App::new(snap, AppOptions::default()), 160, 48);
    assert!(s.contains("No services in payments"), "{s}");
}

#[test]
fn plain_theme_has_no_colors() {
    let mut a = App::new(
        sample_snapshot(),
        AppOptions {
            plain: true,
            ..AppOptions::default()
        },
    );
    let buf = render_to_buffer(&mut a, 160, 48);
    assert!(
        buf.content()
            .iter()
            .all(|c| c.fg == Color::Reset && c.bg == Color::Reset)
    );
}

#[test]
fn all_namespaces_label() {
    let mut a = App::new(
        sample_snapshot(),
        AppOptions {
            all_namespaces: true,
            ..AppOptions::default()
        },
    );
    assert!(render_to_string(&mut a, 160, 48).contains("ns all"));
}

#[test]
fn help_overlay_lists_keys() {
    let mut a = app();
    a.update(key(ratatui::crossterm::event::KeyCode::Char('?')));
    let s = render_to_string(&mut a, 160, 48);
    for needle in ["Help", "zoom viewer", "next container", "toggle follow"] {
        assert!(s.contains(needle), "missing {needle:?}\n{s}");
    }
}
