//! `container inspect` JSON (shared by Docker, Podman and nerdctl) → Kubeglass's model.
//! Containers become single-container pods, compose projects become namespaces,
//! published ports become services and named volumes become storage.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

pub use k8s_openapi::jiff::Timestamp;
use serde_json::Value;

use crate::model::{
    Container, Pod, PodStatus, Service, ServicePort, ServiceType, StorageItem, StorageKind,
};
use crate::text::SafeText;

/// Label kglass adds to each container's pod so its published ports can select it.
pub const CONTAINER_LABEL: &str = "kglass.container";
const PROJECT_LABELS: &[&str] = &["com.docker.compose.project", "io.podman.compose.project"];
const SERVICE_LABELS: &[&str] = &["com.docker.compose.service", "io.podman.compose.service"];
/// Namespace shown for containers outside any compose project.
pub const NO_PROJECT: &str = "-";

fn str_at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter().try_fold(v, |cur, key| cur.get(key))?.as_str()
}

/// Container name without Docker's leading `/`, sanitized.
fn name_of(c: &Value) -> Option<SafeText> {
    let raw = c.get("Name")?.as_str()?;
    Some(SafeText::from(raw.trim_start_matches('/')))
}

fn labels_of(c: &Value) -> BTreeMap<String, String> {
    c.get("Config")
        .and_then(|cfg| cfg.get("Labels"))
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

fn first_label<'a>(labels: &'a BTreeMap<String, String>, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| labels.get(*k).map(String::as_str))
}

/// Compose project of a container, or [`NO_PROJECT`].
pub(crate) fn project_of(c: &Value) -> SafeText {
    SafeText::from(first_label(&labels_of(c), PROJECT_LABELS).unwrap_or(NO_PROJECT))
}

fn age(created: Option<&str>, now: Timestamp) -> Duration {
    created
        .and_then(|s| s.parse::<Timestamp>().ok())
        .and_then(|t| Duration::try_from(now.duration_since(t)).ok())
        .unwrap_or(Duration::ZERO)
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|f| f.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// One container as a single-container pod.
pub fn pod(c: &Value, now: Timestamp) -> Option<Pod> {
    let name = name_of(c)?;
    let state = str_at(c, &["State", "Status"]).unwrap_or("unknown");
    let exit_code = c
        .get("State")
        .and_then(|s| s.get("ExitCode"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let (status, waiting) = match state {
        "running" => (PodStatus::Running, None),
        "restarting" => (PodStatus::Restarting, None),
        "exited" | "dead" | "stopped" if exit_code == 0 && state != "dead" => {
            (PodStatus::Completed, None)
        }
        "exited" | "dead" | "stopped" => (PodStatus::Failed, None),
        other => (PodStatus::Pending, Some(SafeText::from(capitalized(other)))),
    };
    let raw_labels = labels_of(c);
    let project = first_label(&raw_labels, PROJECT_LABELS);
    let service = first_label(&raw_labels, SERVICE_LABELS);
    let mut labels: BTreeMap<SafeText, SafeText> = raw_labels
        .iter()
        .map(|(k, v)| (SafeText::from(k.as_str()), SafeText::from(v.as_str())))
        .collect();
    labels.insert(SafeText::from(CONTAINER_LABEL), name.clone());
    let pod_ip = c
        .get("NetworkSettings")
        .and_then(|n| n.get("Networks"))
        .and_then(Value::as_object)
        .and_then(|nets| {
            nets.values()
                .filter_map(|n| n.get("IPAddress")?.as_str())
                .find(|ip| !ip.is_empty())
        })
        .unwrap_or("<none>");
    let restarts = c
        .get("RestartCount")
        .and_then(Value::as_u64)
        .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX));
    Some(Pod {
        namespace: SafeText::from(project.unwrap_or(NO_PROJECT)),
        status,
        containers: vec![Container {
            name: SafeText::from(service.unwrap_or(name.as_str())),
            image: SafeText::from(str_at(c, &["Config", "Image"]).unwrap_or("")),
            ready: status == PodStatus::Running,
            restarts,
            init: false,
            waiting_reason: waiting,
        }],
        age: age(c.get("Created").and_then(Value::as_str), now),
        node: SafeText::from("local"),
        pod_ip: SafeText::from(pod_ip),
        owner: project.map(|p| SafeText::from(format!("compose/{p}/{}", service.unwrap_or("-")))),
        labels,
        name,
    })
}

/// A container's published host ports as a service; `None` when it publishes none.
pub fn published(c: &Value) -> Option<Service> {
    let name = name_of(c)?;
    let ports = c.get("NetworkSettings")?.get("Ports")?.as_object()?;
    let mut host_ip = None;
    let mut seen = BTreeSet::new();
    for (container_port, bindings) in ports {
        let (target, protocol) = container_port
            .split_once('/')
            .unwrap_or((container_port.as_str(), "tcp"));
        for binding in bindings.as_array().into_iter().flatten() {
            let Some(host_port) = binding
                .get("HostPort")
                .and_then(Value::as_str)
                .and_then(|p| p.parse::<u16>().ok())
            else {
                continue;
            };
            host_ip = host_ip.or_else(|| binding.get("HostIp").and_then(Value::as_str));
            // IPv4 and IPv6 bindings of the same port are one published port.
            seen.insert((host_port, target.to_string(), protocol.to_string()));
        }
    }
    if seen.is_empty() {
        return None;
    }
    let running = str_at(c, &["State", "Status"]) == Some("running");
    Some(Service {
        namespace: project_of(c),
        service_type: ServiceType::Published,
        cluster_ip: SafeText::from(host_ip.filter(|ip| !ip.is_empty()).unwrap_or("0.0.0.0")),
        ports: seen
            .into_iter()
            .map(|(port, target, protocol)| ServicePort {
                port,
                target_port: SafeText::from(target),
                node_port: None,
                protocol: SafeText::from(protocol),
            })
            .collect(),
        selector: BTreeMap::from([(SafeText::from(CONTAINER_LABEL), name.clone())]),
        ready_endpoints: u32::from(running),
        name,
    })
}

/// Named volumes and the containers mounting them. Bind mounts (host paths) are
/// deliberately not listed.
pub fn volumes(containers: &[Value]) -> Vec<StorageItem> {
    let mut by_name: BTreeMap<String, StorageItem> = BTreeMap::new();
    for c in containers {
        let Some(container) = name_of(c) else {
            continue;
        };
        let running = str_at(c, &["State", "Status"]) == Some("running");
        for mount in c
            .get("Mounts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if mount.get("Type").and_then(Value::as_str) != Some("volume") {
                continue;
            }
            let Some(name) = mount.get("Name").and_then(Value::as_str) else {
                continue;
            };
            let item = by_name
                .entry(name.to_string())
                .or_insert_with(|| StorageItem {
                    kind: StorageKind::Volume,
                    name: SafeText::from(name),
                    status: SafeText::from("Idle"),
                    capacity: SafeText::from("—"),
                    access_modes: SafeText::from(
                        if mount.get("RW").and_then(Value::as_bool).unwrap_or(true) {
                            "RW"
                        } else {
                            "RO"
                        },
                    ),
                    storage_class: SafeText::from(
                        mount
                            .get("Driver")
                            .and_then(Value::as_str)
                            .unwrap_or("local"),
                    ),
                    bound_to: None,
                    mounted_by: Vec::new(),
                });
            if running {
                item.status = SafeText::from("In use");
            }
            if !item.mounted_by.contains(&container) {
                item.mounted_by.push(container.clone());
            }
        }
    }
    by_name.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PodStatus, ServiceType, StorageKind, format_age, format_ports};

    fn fixture() -> Vec<Value> {
        serde_json::from_str(include_str!("../../tests/fixtures/oci-inspect.json")).unwrap()
    }

    fn now() -> Timestamp {
        "2026-11-07T12:00:00Z".parse().unwrap()
    }

    fn pods() -> Vec<Pod> {
        fixture().iter().filter_map(|c| pod(c, now())).collect()
    }

    fn named<'a>(pods: &'a [Pod], name: &str) -> &'a Pod {
        pods.iter().find(|p| p.name.as_str() == name).unwrap()
    }

    #[test]
    fn containers_become_pods() {
        let all = pods();
        let web = named(&all, "shop-web-1");
        assert_eq!(web.status, PodStatus::Running);
        assert_eq!(web.namespace.as_str(), "shop");
        assert_eq!(web.ready(), (1, 1));
        assert_eq!(format_age(web.age), "2h00m");
        assert_eq!(web.pod_ip.as_str(), "172.17.0.2");
        assert_eq!(web.node.as_str(), "local");
        assert_eq!(web.owner.as_ref().unwrap().as_str(), "compose/shop/web");
        assert_eq!(web.containers[0].name.as_str(), "web");
        assert_eq!(web.containers[0].image.as_str(), "nginx:1.27");
        assert_eq!(
            web.labels
                .get(&SafeText::from(CONTAINER_LABEL))
                .map(SafeText::as_str),
            Some("shop-web-1")
        );
    }

    #[test]
    fn engine_states_map_to_statuses() {
        let all = pods();
        let status = |n| named(&all, n).status;
        assert_eq!(status("shop-worker-1"), PodStatus::Failed);
        assert_eq!(named(&all, "shop-worker-1").restarts(), 3);
        assert_eq!(status("migrate"), PodStatus::Completed);
        assert_eq!(
            named(&all, "migrate").namespace.as_str(),
            "-",
            "no compose project"
        );
        assert_eq!(status("flaky"), PodStatus::Restarting);
        assert_eq!(
            named(&all, "flaky").namespace.as_str(),
            "lab",
            "podman-compose label"
        );
        assert_eq!(status("fresh"), PodStatus::Pending);
        assert_eq!(named(&all, "fresh").status_label(), "Pending");
        assert_eq!(
            named(&all, "fresh").containers[0]
                .waiting_reason
                .as_ref()
                .map(SafeText::as_str),
            Some("Created")
        );
    }

    #[test]
    fn names_are_sanitized() {
        assert!(pods().iter().any(|p| p.name.as_str() == "evil-box"));
    }

    #[test]
    fn published_ports_become_services() {
        let raw = fixture();
        let services: Vec<_> = raw.iter().filter_map(published).collect();
        assert_eq!(services.len(), 1, "only containers publishing host ports");
        let web = &services[0];
        assert_eq!(web.name.as_str(), "shop-web-1");
        assert_eq!(web.service_type, ServiceType::Published);
        assert_eq!(
            format_ports(&web.ports),
            "8080→80/tcp",
            "IPv4/IPv6 bindings of one port count once"
        );
        assert_eq!(web.cluster_ip.as_str(), "0.0.0.0");
        assert_eq!(web.ready_endpoints, 1);
        let all = pods();
        let matched = crate::derive::pods_matching(&web.selector, &web.namespace, &all);
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].name.as_str(), "shop-web-1");
    }

    #[test]
    fn named_volumes_with_their_containers_and_no_bind_mounts() {
        let vols = volumes(&fixture());
        assert_eq!(vols.len(), 1, "bind mounts are not listed");
        let data = &vols[0];
        assert_eq!(data.kind, StorageKind::Volume);
        assert_eq!(data.name.as_str(), "shop-data");
        assert_eq!(data.status.as_str(), "In use");
        assert_eq!(data.storage_class.as_str(), "local");
        let users: Vec<&str> = data.mounted_by.iter().map(SafeText::as_str).collect();
        assert_eq!(users, ["shop-web-1", "shop-worker-1"]);
        assert!(
            !format!("{vols:?}").contains("/Users/someone"),
            "host paths never shown"
        );
    }

    #[test]
    fn non_object_input_is_skipped() {
        assert!(pod(&serde_json::json!("nope"), now()).is_none());
        assert!(published(&serde_json::json!(42)).is_none());
    }
}
