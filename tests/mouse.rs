mod support;

use kglass::app::{AppEvent, Focus, ViewerTab};
use kglass::model::{ResourceKind, ResourceRef};
use ratatui::crossterm::event::KeyCode;
use support::*;

#[test]
fn click_row_selects_it() {
    let mut a = app();
    render_to_string(&mut a, 160, 48);
    let (rect, _, _) = *a
        .hits
        .rows
        .iter()
        .find(|(_, k, i)| *k == ResourceKind::Services && *i == 2)
        .expect("billing row is visible");
    a.update(click(rect.x + 1, rect.y));
    assert_eq!(a.selected, Some(ResourceRef::Service("billing".into())));
    assert_eq!(a.focus, Focus::List(ResourceKind::Services));
    assert_eq!(
        a.cursors[ResourceKind::Services.index()].selected(),
        Some(2)
    );
}

#[test]
fn click_viewer_tab_switches() {
    let mut a = app();
    render_to_string(&mut a, 160, 48);
    let (rect, _) = *a.hits.tabs.iter().find(|(_, i)| *i == 3).expect("YAML tab");
    a.update(click(rect.x, rect.y));
    assert_eq!(a.current_tab(), Some(ViewerTab::Yaml));
}

#[test]
fn click_viewer_body_focuses_viewer() {
    let mut a = app();
    render_to_string(&mut a, 160, 48);
    let v = a.hits.viewer;
    a.update(click(v.x + v.width / 2, v.y + v.height / 2));
    assert_eq!(a.focus, Focus::Viewer);
}

#[test]
fn wheel_over_list_moves_cursor() {
    let mut a = app();
    render_to_string(&mut a, 160, 48);
    let (rect, _) = *a
        .hits
        .panes
        .iter()
        .find(|(_, k)| *k == ResourceKind::Images)
        .unwrap();
    a.update(wheel(rect.x + 2, rect.y + 2, true));
    assert_eq!(a.cursors[ResourceKind::Images.index()].selected(), Some(3));
    assert_eq!(
        a.focus,
        Focus::List(ResourceKind::Pods),
        "wheel does not steal focus"
    );
}

#[test]
fn click_narrow_list_tab() {
    let mut a = app();
    render_to_string(&mut a, 92, 48);
    let (rect, _) = *a
        .hits
        .list_tabs
        .iter()
        .find(|(_, k)| *k == ResourceKind::Storage)
        .unwrap();
    a.update(click(rect.x + 1, rect.y));
    assert_eq!(a.focus, Focus::List(ResourceKind::Storage));
    assert!(render_to_string(&mut a, 92, 48).contains("data-billing-0"));
}

#[test]
fn click_on_nothing_is_ignored() {
    let mut a = app();
    render_to_string(&mut a, 160, 48);
    let before = (a.focus, a.selected.clone());
    a.update(click(0, 0)); // header
    a.update(click(500, 500)); // outside the screen
    assert_eq!((a.focus, a.selected.clone()), before);
}

#[test]
fn resize_to_narrow_keeps_focus_and_zoom() {
    let mut a = app();
    a.update(key(KeyCode::Char('4')));
    a.update(key(KeyCode::Char('z')));
    render_to_string(&mut a, 160, 48);
    a.update(AppEvent::Resize(92, 48));
    let s = render_to_string(&mut a, 92, 48);
    assert_eq!(a.focus, Focus::Viewer, "zoom focuses the viewer");
    assert!(a.zoom);
    assert!(s.contains("Pod api-7c9d8f-x2k4"), "{s}");
    a.update(key(KeyCode::Char('z')));
    assert!(render_to_string(&mut a, 92, 48).contains("data-billing-0"));
}
