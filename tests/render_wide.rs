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

fn select_row(a: &mut App, list: char, row: usize) {
    use ratatui::crossterm::event::KeyCode;
    a.update(key(KeyCode::Char(list)));
    for _ in 0..row {
        a.update(key(KeyCode::Down));
    }
    a.update(key(KeyCode::Enter));
}

fn tab_right(a: &mut App, n: usize) {
    for _ in 0..n {
        a.update(key(ratatui::crossterm::event::KeyCode::Right));
    }
}

#[test]
fn pod_logs_toolbar_and_status() {
    let s = render_to_string(&mut app(), 160, 48);
    for needle in [
        "Logs - Summary - Events - YAML - Containers",
        "Pod api-7c9d8f-x2k4",
        "container ▸ api",
        "envoy",
        "follow ● on",
        "tail 500",
        "34 lines buffered (cap 5000)",
        "12:04:11.000 INFO",
        "12:04:25.421",
    ] {
        assert!(s.contains(needle), "missing {needle:?}\n{s}");
    }
}

#[test]
fn follow_off_is_shown() {
    let mut a = app();
    a.update(key(ratatui::crossterm::event::KeyCode::Char('f')));
    assert!(render_to_string(&mut a, 160, 48).contains("follow ○ off"));
}

#[test]
fn pending_pod_explains_instead_of_empty_logs() {
    let mut a = app();
    select_row(&mut a, '1', 5);
    let s = render_to_string(&mut a, 160, 48);
    assert!(
        s.contains("Container ledger-sync has not started (ContainerCreating)"),
        "{s}"
    );
    assert!(
        s.contains("waiting for PVC ledger-cache"),
        "pod events are shown\n{s}"
    );
}

#[test]
fn pod_summary_containers_events_yaml() {
    let mut a = app();
    select_row(&mut a, '1', 3);
    tab_right(&mut a, 1);
    let s = render_to_string(&mut a, 160, 48);
    assert!(
        s.contains("StatefulSet/billing") && s.contains("app=billing"),
        "summary\n{s}"
    );
    tab_right(&mut a, 1);
    let s = render_to_string(&mut a, 160, 48);
    assert!(
        s.contains("BackOff") && s.contains("Back-off restarting"),
        "events\n{s}"
    );
    tab_right(&mut a, 1);
    let s = render_to_string(&mut a, 160, 48);
    assert!(
        s.contains("kind: Pod") && s.contains("initContainers:"),
        "yaml\n{s}"
    );
    tab_right(&mut a, 1);
    let s = render_to_string(&mut a, 160, 48);
    assert!(
        s.contains("init-perms") && s.contains("[init]") && s.contains("busybox:1.36"),
        "containers\n{s}"
    );
}

#[test]
fn events_tab_without_events() {
    let mut a = app();
    tab_right(&mut a, 2);
    assert!(render_to_string(&mut a, 160, 48).contains("No events for this object"));
}

#[test]
fn image_details_and_workloads() {
    let mut a = app();
    select_row(&mut a, '2', 0);
    let s = render_to_string(&mut a, 160, 48);
    assert!(s.contains("Image envoyproxy/envoy:v1.31.2"), "{s}");
    assert!(s.contains("Registry metadata: not retrieved"), "{s}");
    tab_right(&mut a, 1);
    let s = render_to_string(&mut a, 160, 48);
    assert!(
        s.contains("api-7c9d8f-x2k4 → envoy") && s.contains("api-7c9d8f-q8n1 → envoy"),
        "{s}"
    );
}

#[test]
fn pvc_mounted_by_is_derived() {
    let mut a = app();
    select_row(&mut a, '4', 0);
    let s = render_to_string(&mut a, 160, 48);
    assert!(
        s.contains("PVC data-billing-0") && s.contains("pvc-3e1a9c52-0c09"),
        "{s}"
    );
    assert!(s.contains("Kubernetes metadata only"), "{s}");
    tab_right(&mut a, 1);
    let s = render_to_string(&mut a, 160, 48);
    assert!(s.contains("billing-0") && s.contains("[derived]"), "{s}");
}

#[test]
fn released_pv_explains() {
    let mut a = app();
    select_row(&mut a, '4', 4);
    let s = render_to_string(&mut a, 160, 48);
    assert!(s.contains("Released — claim was deleted"), "{s}");
}

#[test]
fn yaml_scrolls_in_viewer() {
    let mut a = app();
    select_row(&mut a, '3', 4);
    tab_right(&mut a, 3);
    // Services → Storage → Viewer
    a.update(key(ratatui::crossterm::event::KeyCode::Tab));
    a.update(key(ratatui::crossterm::event::KeyCode::Tab));
    assert_eq!(a.focus, kglass::app::Focus::Viewer);
    let before = render_to_string(&mut a, 160, 48);
    assert!(before.contains("apiVersion: v1"), "{before}");
    for _ in 0..3 {
        a.update(key(ratatui::crossterm::event::KeyCode::Down));
    }
    let after = render_to_string(&mut a, 160, 48);
    assert!(!after.contains("apiVersion: v1"), "{after}");
}

#[test]
#[ignore = "prints the screens for manual comparison with the design PDF"]
fn print_screens() {
    let mut a = app();
    println!("{}", render_to_string(&mut a, 160, 48));
    a.update(key(ratatui::crossterm::event::KeyCode::Char('3')));
    a.update(key(ratatui::crossterm::event::KeyCode::Down));
    a.update(key(ratatui::crossterm::event::KeyCode::Down));
    a.update(key(ratatui::crossterm::event::KeyCode::Enter));
    println!("{}", render_to_string(&mut a, 92, 48));
}

#[test]
fn header_says_sample_data_not_connected() {
    let wide = render_to_string(&mut app(), 160, 48);
    let header = wide.lines().next().unwrap();
    assert!(
        header.contains("SAMPLE DATA · not connected to a cluster"),
        "{header}"
    );
    assert!(
        !header.contains("Connected"),
        "must not claim a connection: {header}"
    );
    let narrow = render_to_string(&mut app(), 92, 48);
    let header = narrow.lines().next().unwrap();
    assert!(header.contains("SAMPLE DATA"), "{header}");
    assert!(!header.contains("Connected"), "{header}");
}
