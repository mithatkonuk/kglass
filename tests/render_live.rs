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

fn with_contexts(mut a: kglass::app::App) -> kglass::app::App {
    use kglass::cluster::config::ContextInfo;
    let info = |name: &str, hint: &'static str| ContextInfo {
        name: name.into(),
        cluster: name.into(),
        server_host: "k8s.example.test".into(),
        namespace: None,
        hint,
    };
    a.set_contexts(vec![
        info("dev-01", "rancher"),
        info("dev-02", "rancher"),
        info("kind-local", "local"),
    ]);
    a
}

#[test]
fn context_picker_overlay() {
    let mut a = with_contexts(live_app());
    a.update(key(KeyCode::Char('x')));
    let s = render_to_string(&mut a, 160, 48);
    for needle in [
        "[x]─Switch context",
        "filter › █",
        "● dev-01",
        "○ kind-local",
        "rancher · k8s.example.test",
        "3 of 3 contexts · from kubeconfig · enter switch · esc cancel",
    ] {
        assert!(s.contains(needle), "missing {needle:?}\n{s}");
    }
    for c in "kind".chars() {
        a.update(key(KeyCode::Char(c)));
    }
    let s = render_to_string(&mut a, 160, 48);
    assert!(
        s.contains("filter › kind█") && s.contains("1 of 3 contexts"),
        "{s}"
    );
}

#[test]
fn namespace_picker_loading_and_error() {
    let mut a = live_app();
    a.update(key(KeyCode::Char('n')));
    let s = render_to_string(&mut a, 160, 48);
    assert!(
        s.contains("[n]─Switch namespace") && s.contains("loading namespaces…"),
        "{s}"
    );
    a.update(AppEvent::Data(DataEvent::Namespaces(Err(
        FetchError::Timeout,
    ))));
    let s = render_to_string(&mut a, 160, 48);
    assert!(
        s.contains("✕ cannot list namespaces: timed out after 15s"),
        "{s}"
    );
    assert!(s.contains("type a namespace name and press enter"), "{s}");
}

#[test]
fn picker_fits_narrow_minimum() {
    let mut a = with_contexts(live_app());
    a.update(key(KeyCode::Char('x')));
    let s = render_to_string(&mut a, 60, 16);
    assert!(
        s.contains("Switch context") && s.contains("dev-01"),
        "{s}"
    );
}

#[test]
fn filter_shown_in_title_and_footer() {
    let mut a = live_app();
    a.update(lists_event(sample_lists()));
    a.update(key(KeyCode::Char('/')));
    for c in "bill".chars() {
        a.update(key(KeyCode::Char(c)));
    }
    let s = render_to_string(&mut a, 160, 48);
    assert!(s.contains("[1]─Pods 1/6 · filter: bill"), "{s}");
    let footer = s.lines().last().unwrap();
    assert!(
        footer.contains("filter pods › bill█") && footer.contains("enter done · esc clear"),
        "{footer}"
    );
    a.update(key(KeyCode::Enter));
    let s = render_to_string(&mut a, 160, 48);
    assert!(
        s.contains("[1]─Pods 1/6 · filter: bill"),
        "filter stays after enter\n{s}"
    );
}

#[test]
fn live_footer_lists_cluster_keys() {
    let mut a = live_app();
    let footer = render_to_string(&mut a, 160, 48)
        .lines()
        .last()
        .unwrap()
        .to_string();
    for needle in ["x context", "n namespace", "/ filter", "r refresh"] {
        assert!(footer.contains(needle), "missing {needle:?}: {footer}");
    }
}
