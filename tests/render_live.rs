mod support;

use kglass::app::{AppEvent, DataEvent};
use kglass::cluster::fetch::FetchError;
use kglass::model::ResourceKind;
use ratatui::crossterm::event::KeyCode;
use support::*;

fn header(s: &str) -> &str {
    s.lines().next().unwrap()
}

#[test]
fn header_shows_real_context_and_version() {
    let mut a = live_app();
    a.update(AppEvent::Data(DataEvent::Version {
        generation: 0,
        result: Ok("v1.36.4+rke2r1".into()),
    }));
    a.update(lists_event(sample_lists()));
    let s = render_to_string(&mut a, 160, 48);
    let h = header(&s);
    for needle in [
        "ctx dev-01",
        "ns product-test",
        "● Connected",
        "api v1.36.4+rke2r1",
        "every 10s",
    ] {
        assert!(h.contains(needle), "missing {needle:?}: {h}");
    }
    assert!(!h.contains("SAMPLE"), "{h}");
}

#[test]
fn connecting_and_loading_states() {
    let mut a = live_app();
    for _ in 0..8 {
        a.update(AppEvent::Tick);
    }
    let s = render_to_string(&mut a, 160, 48);
    assert!(header(&s).contains("Connecting"), "{}", header(&s));
    assert!(s.contains("loading pods… (2s)"), "{s}");
    assert!(s.contains("loading services…"), "{s}");
    assert!(s.contains("Nothing selected"), "{s}");
}

#[test]
fn disconnected_header() {
    let mut a = live_app();
    a.update(AppEvent::Data(DataEvent::Version {
        generation: 0,
        result: Err(FetchError::Unauthorized("token expired".into())),
    }));
    let h = render_to_string(&mut a, 160, 48)
        .lines()
        .next()
        .unwrap()
        .to_string();
    assert!(
        h.contains("● Disconnected") && h.contains("token expired"),
        "{h}"
    );
}

#[test]
fn forbidden_pane_message() {
    let mut a = live_app();
    let mut l = sample_lists();
    l.storage = Err(FetchError::Forbidden {
        verb: "list",
        resource: "persistentvolumeclaims",
        scope: "namespace \"product-test\"".into(),
        message: "forbidden".into(),
    });
    a.update(lists_event(l));
    let s = render_to_string(&mut a, 160, 48);
    for needle in [
        "✕ Forbidden (403)",
        "cannot list persistentvolumeclaims",
        "Needed (read-only): persistentvolumeclaims: get, list",
    ] {
        assert!(s.contains(needle), "missing {needle:?}\n{s}");
    }
    assert!(s.contains("api-7c9d8f-x2k4"), "other panes still render");
}

#[test]
fn partial_storage_note() {
    let mut a = live_app();
    let mut l = sample_lists();
    l.storage_partial = Some(FetchError::Forbidden {
        verb: "list",
        resource: "persistentvolumes",
        scope: "cluster".into(),
        message: "forbidden".into(),
    });
    a.update(lists_event(l));
    let s = render_to_string(&mut a, 160, 48);
    assert!(s.contains("data-billing-0"), "{s}");
    assert!(s.contains("PVs hidden: Forbidden (403)"), "{s}");
}

#[test]
fn stale_title_after_failed_refresh() {
    let mut a = live_app();
    a.update(lists_event(sample_lists()));
    a.update(key(KeyCode::Char('r')));
    let mut l = sample_lists();
    l.pods = Err(FetchError::Timeout);
    a.update(lists_event(l));
    let s = render_to_string(&mut a, 160, 48);
    assert!(s.contains("[1]─Pods 6 · STALE"), "{s}");
    assert!(s.contains("api-7c9d8f-x2k4"), "last rows still shown\n{s}");
    assert!(a.is_stale(ResourceKind::Pods));
}

#[test]
fn disappeared_notice() {
    let mut a = live_app();
    a.update(lists_event(sample_lists()));
    let mut l = sample_lists();
    if let Ok(pods) = &mut l.pods {
        pods.remove(0);
    }
    a.update(key(KeyCode::Char('r')));
    a.update(lists_event(l));
    let s = render_to_string(&mut a, 160, 48);
    assert!(
        s.contains("⊘ Pod api-7c9d8f-x2k4 is no longer present"),
        "{s}"
    );
}

#[test]
fn live_logs_status_and_yaml_states() {
    let mut a = live_app();
    a.update(lists_event(sample_lists()));
    a.update(AppEvent::Data(DataEvent::LogLine {
        session: 1,
        line: "real log line".into(),
    }));
    let s = render_to_string(&mut a, 160, 48);
    assert!(
        s.contains("real log line") && s.contains("● streaming · 1 lines buffered"),
        "{s}"
    );
    assert!(!s.contains("(sample)"), "{s}");
    a.update(AppEvent::Data(DataEvent::LogEnded {
        session: 1,
        error: Some(FetchError::Unreachable("connection reset".into())),
    }));
    assert!(
        render_to_string(&mut a, 160, 48)
            .contains("○ stream ended: cannot reach the cluster: connection reset")
    );
    for _ in 0..3 {
        a.update(key(KeyCode::Right));
    }
    assert!(render_to_string(&mut a, 160, 48).contains("loading YAML…"));
    a.update(AppEvent::Data(DataEvent::Yaml {
        generation: 0,
        target: kglass::model::ResourceRef::Pod("api-7c9d8f-x2k4".into()),
        result: Err(FetchError::Timeout),
    }));
    assert!(render_to_string(&mut a, 160, 48).contains("✕ timed out after 15s"));
}

#[test]
fn live_context_box_without_user() {
    let mut a = live_app();
    let s = render_to_string(&mut a, 160, 48);
    assert!(
        s.contains("context dev-01  namespace product-test"),
        "{s}"
    );
    assert!(!s.contains("user "), "{s}");
}
