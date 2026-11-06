mod support;

use kglass::ui::layout::{LayoutMode, layout_mode};
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;
use support::*;

#[test]
fn narrow_matches_pdf_page2_text() {
    let mut a = app();
    a.update(key(KeyCode::Char('3')));
    let s = render_to_string(&mut a, 92, 48);
    for needle in [
        "KG",
        "RO",
        "[1] Pods 6",
        "[2] Images 7",
        "[3] Services 5",
        "[4] Storage 5",
        "payments-lb",
        "80→8080/TCP",
    ] {
        assert!(s.contains(needle), "missing {needle:?}\n{s}");
    }
    assert!(
        !s.contains("api-7c9d8f-q8n1"),
        "only the focused list is shown\n{s}"
    );
    assert!(
        !s.contains("10.96.14.22"),
        "narrow services drop CLUSTER-IP\n{s}"
    );
}

#[test]
fn layout_modes() {
    assert_eq!(layout_mode(Rect::new(0, 0, 160, 48)), LayoutMode::Wide);
    assert_eq!(layout_mode(Rect::new(0, 0, 120, 30)), LayoutMode::Wide);
    assert_eq!(layout_mode(Rect::new(0, 0, 119, 48)), LayoutMode::Narrow);
    assert_eq!(layout_mode(Rect::new(0, 0, 160, 29)), LayoutMode::Narrow);
    assert_eq!(layout_mode(Rect::new(0, 0, 92, 48)), LayoutMode::Narrow);
    assert_eq!(layout_mode(Rect::new(0, 0, 59, 48)), LayoutMode::TooSmall);
    assert_eq!(layout_mode(Rect::new(0, 0, 92, 15)), LayoutMode::TooSmall);
}

#[test]
fn narrow_shows_last_list_when_viewer_focused() {
    let mut a = app();
    a.update(key(KeyCode::Char('4')));
    a.update(key(KeyCode::Tab));
    let s = render_to_string(&mut a, 92, 48);
    assert!(s.contains("data-billing-0"), "{s}");
}

#[test]
fn minimum_narrow_size_renders() {
    let s = render_to_string(&mut app(), 60, 16);
    assert!(s.contains("[1] Pods 6"), "{s}");
}

#[test]
fn service_billing_matches_pdf_page2() {
    let mut a = app();
    a.update(key(KeyCode::Char('3')));
    a.update(key(KeyCode::Down));
    a.update(key(KeyCode::Down));
    a.update(key(KeyCode::Enter));
    let s = render_to_string(&mut a, 92, 48);
    for needle in [
        "Details - Endpoints - Events - YAML",
        "Service billing",
        "app=billing",
        "8443/TCP",
        "No ready endpoints.",
        "1 pod matches the selector; 0 are Ready.",
        "→ billing-0",
        "CrashLoopBackOff",
        "[derived]",
        "not network reachability",
    ] {
        assert!(s.contains(needle), "missing {needle:?}\n{s}");
    }
}

#[test]
fn service_endpoints_tab() {
    let mut a = app();
    a.update(key(KeyCode::Char('3')));
    a.update(key(KeyCode::Enter));
    a.update(key(KeyCode::Right));
    let s = render_to_string(&mut a, 92, 48);
    assert!(s.contains("2 ready endpoints"), "{s}");
    assert!(s.contains("10.244.1.17") && s.contains("[derived]"), "{s}");
}

#[test]
fn help_fits_minimum_terminal() {
    let mut a = app();
    a.update(key(KeyCode::Char('?')));
    let s = render_to_string(&mut a, 60, 16);
    for needle in ["Help", "quit", "wheel scrolls"] {
        assert!(s.contains(needle), "missing {needle:?}\n{s}");
    }
}

#[test]
fn related_cursor_scrolls_into_view() {
    let mut a = app();
    for _ in 0..3 {
        a.update(key(KeyCode::Down));
    }
    a.update(key(KeyCode::Enter));
    a.update(key(KeyCode::Right));
    while a.focus != kglass::app::Focus::Viewer {
        a.update(key(KeyCode::Tab));
    }
    let before = render_to_string(&mut a, 92, 20);
    assert!(
        !before.contains("PVC data-billing-0"),
        "jump section starts below the fold\n{before}"
    );
    a.update(key(KeyCode::Down));
    let after = render_to_string(&mut a, 92, 20);
    assert!(
        after.contains("→ PVC data-billing-0"),
        "highlighted item scrolled into view\n{after}"
    );
}
