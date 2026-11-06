//! Conversion from Kubernetes API objects to Kubeglass's model.
//! Every cluster string becomes `SafeText` here, at the boundary.

use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

use k8s_openapi::api::core::v1 as k8s;
pub use k8s_openapi::api::discovery::v1::EndpointSlice;
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
pub use k8s_openapi::jiff::Timestamp;
use serde::Serialize;

use crate::model::{
    Container, EventType, KubeEvent, Pod, PodStatus, ResourceRef, Service, ServicePort,
    ServiceType, StorageItem, StorageKind,
};
use crate::text::SafeText;

/// Container waiting reasons that mean the pod cannot start (shown as `Failing`).
const FAILING_REASONS: &[&str] = &[
    "ImagePullBackOff",
    "ErrImagePull",
    "ErrImageNeverPull",
    "InvalidImageName",
    "CreateContainerConfigError",
    "CreateContainerError",
    "RunContainerError",
];

const SERVICE_NAME_LABEL: &str = "kubernetes.io/service-name";

fn text(s: Option<&str>, fallback: &str) -> SafeText {
    SafeText::from(s.unwrap_or(fallback))
}

/// Time since `ts`; zero when missing or in the future (clock skew).
fn age(ts: Option<Timestamp>, now: Timestamp) -> Duration {
    ts.and_then(|t| Duration::try_from(now.duration_since(t)).ok())
        .unwrap_or(Duration::ZERO)
}

fn labels(map: Option<&BTreeMap<String, String>>) -> BTreeMap<SafeText, SafeText> {
    map.into_iter()
        .flatten()
        .map(|(k, v)| (SafeText::from(k.as_str()), SafeText::from(v.as_str())))
        .collect()
}

fn containers(
    specs: &[k8s::Container],
    statuses: Option<&Vec<k8s::ContainerStatus>>,
    init: bool,
) -> Vec<Container> {
    specs
        .iter()
        .map(|spec| {
            let status = statuses.and_then(|all| all.iter().find(|s| s.name == spec.name));
            let waiting = status
                .and_then(|s| s.state.as_ref())
                .and_then(|st| st.waiting.as_ref())
                .and_then(|w| w.reason.as_deref());
            Container {
                name: SafeText::from(spec.name.as_str()),
                image: text(spec.image.as_deref(), ""),
                ready: status.is_some_and(|s| s.ready),
                restarts: status.map_or(0, |s| u32::try_from(s.restart_count).unwrap_or(0)),
                init,
                waiting_reason: waiting.map(SafeText::from),
            }
        })
        .collect()
}

fn pod_status(p: &k8s::Pod, containers: &[Container]) -> PodStatus {
    if p.metadata.deletion_timestamp.is_some() {
        return PodStatus::Terminating;
    }
    let phase = p.status.as_ref().and_then(|s| s.phase.as_deref());
    match phase {
        Some("Succeeded") => return PodStatus::Completed,
        Some("Failed") => return PodStatus::Failed,
        _ => {}
    }
    let reasons: Vec<&str> = containers
        .iter()
        .filter_map(|c| c.waiting_reason.as_ref().map(SafeText::as_str))
        .collect();
    if reasons.contains(&"CrashLoopBackOff") {
        PodStatus::CrashLoopBackOff
    } else if reasons.iter().any(|r| FAILING_REASONS.contains(r)) {
        PodStatus::Failing
    } else if phase == Some("Running") {
        PodStatus::Running
    } else {
        PodStatus::Pending
    }
}

pub fn pod(p: &k8s::Pod, now: Timestamp) -> Pod {
    let spec = p.spec.as_ref();
    let status = p.status.as_ref();
    let mut all = containers(
        spec.and_then(|s| s.init_containers.as_deref())
            .unwrap_or_default(),
        status.and_then(|s| s.init_container_statuses.as_ref()),
        true,
    );
    all.extend(containers(
        spec.map(|s| s.containers.as_slice()).unwrap_or_default(),
        status.and_then(|s| s.container_statuses.as_ref()),
        false,
    ));
    let owners = p.metadata.owner_references.as_deref().unwrap_or_default();
    let owner = owners
        .iter()
        .find(|o| o.controller == Some(true))
        .or_else(|| owners.first())
        .map(|o| SafeText::from(format!("{}/{}", o.kind, o.name)));
    Pod {
        name: text(p.metadata.name.as_deref(), ""),
        namespace: text(p.metadata.namespace.as_deref(), ""),
        status: pod_status(p, &all),
        labels: labels(p.metadata.labels.as_ref()),
        containers: all,
        age: age(p.metadata.creation_timestamp.as_ref().map(|t| t.0), now),
        node: text(spec.and_then(|s| s.node_name.as_deref()), "<none>"),
        pod_ip: text(status.and_then(|s| s.pod_ip.as_deref()), "<none>"),
        owner,
    }
}

fn port_number(n: i32) -> u16 {
    u16::try_from(n).unwrap_or(0)
}

pub fn service(s: &k8s::Service, slices: &[EndpointSlice]) -> Service {
    let spec = s.spec.as_ref();
    let name = s.metadata.name.as_deref().unwrap_or("");
    let service_type = match spec.and_then(|sp| sp.type_.as_deref()) {
        Some("NodePort") => ServiceType::NodePort,
        Some("LoadBalancer") => ServiceType::LoadBalancer,
        Some("ExternalName") => ServiceType::ExternalName,
        _ => ServiceType::ClusterIP,
    };
    let ports = spec
        .and_then(|sp| sp.ports.as_deref())
        .unwrap_or_default()
        .iter()
        .map(|p| ServicePort {
            port: port_number(p.port),
            target_port: match &p.target_port {
                Some(IntOrString::Int(n)) => SafeText::from(n.to_string()),
                Some(IntOrString::String(s)) => SafeText::from(s.as_str()),
                None => SafeText::from(p.port.to_string()),
            },
            node_port: p.node_port.map(port_number),
            protocol: text(p.protocol.as_deref(), "TCP"),
        })
        .collect();
    // Dual-stack Services have one EndpointSlice per address family, so the same
    // pod appears twice; count each backing pod (or address) once.
    let ready_endpoints: HashSet<String> = slices
        .iter()
        .filter(|sl| {
            sl.metadata
                .labels
                .as_ref()
                .and_then(|l| l.get(SERVICE_NAME_LABEL))
                .is_some_and(|n| n == name)
        })
        .flat_map(|sl| sl.endpoints.iter().flatten())
        // An unset `ready` condition should be treated as ready (EndpointConditions docs).
        .filter(|ep| ep.conditions.as_ref().and_then(|c| c.ready) != Some(false))
        .filter_map(|ep| match ep.target_ref.as_ref() {
            Some(r) => Some(format!(
                "{}/{}",
                r.kind.as_deref().unwrap_or(""),
                r.name.as_deref().unwrap_or("")
            )),
            None => ep.addresses.first().cloned(),
        })
        .collect();
    Service {
        name: SafeText::from(name),
        namespace: text(s.metadata.namespace.as_deref(), ""),
        service_type,
        cluster_ip: text(spec.and_then(|sp| sp.cluster_ip.as_deref()), "None"),
        ports,
        selector: labels(spec.and_then(|sp| sp.selector.as_ref())),
        ready_endpoints: u32::try_from(ready_endpoints.len()).unwrap_or(u32::MAX),
    }
}

fn access_modes(modes: Option<&Vec<String>>) -> SafeText {
    let short: Vec<&str> = modes
        .into_iter()
        .flatten()
        .map(|m| match m.as_str() {
            "ReadWriteOnce" => "RWO",
            "ReadOnlyMany" => "ROX",
            "ReadWriteMany" => "RWX",
            "ReadWriteOncePod" => "RWOP",
            other => other,
        })
        .collect();
    SafeText::from(if short.is_empty() {
        "—".to_string()
    } else {
        short.join(",")
    })
}

fn storage_quantity(map: Option<&BTreeMap<String, Quantity>>) -> Option<&str> {
    map.and_then(|m| m.get("storage")).map(|q| q.0.as_str())
}

/// `pods` are the raw pods of the same namespace, used to find which pods mount the claim.
pub fn pvc(c: &k8s::PersistentVolumeClaim, pods: &[k8s::Pod]) -> StorageItem {
    let name = c.metadata.name.as_deref().unwrap_or("");
    let spec = c.spec.as_ref();
    let status = c.status.as_ref();
    let capacity = storage_quantity(status.and_then(|s| s.capacity.as_ref())).or_else(|| {
        storage_quantity(
            spec.and_then(|s| s.resources.as_ref())
                .and_then(|r| r.requests.as_ref()),
        )
    });
    let mounted_by = pods
        .iter()
        .filter(|p| {
            p.spec
                .as_ref()
                .and_then(|s| s.volumes.as_deref())
                .unwrap_or_default()
                .iter()
                .any(|v| {
                    v.persistent_volume_claim
                        .as_ref()
                        .is_some_and(|src| src.claim_name == name)
                })
        })
        .map(|p| text(p.metadata.name.as_deref(), ""))
        .collect();
    StorageItem {
        kind: StorageKind::Pvc,
        name: SafeText::from(name),
        status: text(status.and_then(|s| s.phase.as_deref()), "Unknown"),
        capacity: text(capacity, "—"),
        access_modes: access_modes(spec.and_then(|s| s.access_modes.as_ref())),
        storage_class: text(spec.and_then(|s| s.storage_class_name.as_deref()), "—"),
        bound_to: spec
            .and_then(|s| s.volume_name.as_deref())
            .map(SafeText::from),
        mounted_by,
    }
}

pub fn pv(v: &k8s::PersistentVolume) -> StorageItem {
    let spec = v.spec.as_ref();
    let bound_to = spec.and_then(|s| s.claim_ref.as_ref()).map(|r| {
        SafeText::from(format!(
            "{}/{}",
            r.namespace.as_deref().unwrap_or(""),
            r.name.as_deref().unwrap_or("")
        ))
    });
    StorageItem {
        kind: StorageKind::Pv,
        name: text(v.metadata.name.as_deref(), ""),
        status: text(
            v.status.as_ref().and_then(|s| s.phase.as_deref()),
            "Unknown",
        ),
        capacity: text(
            storage_quantity(spec.and_then(|s| s.capacity.as_ref())),
            "—",
        ),
        access_modes: access_modes(spec.and_then(|s| s.access_modes.as_ref())),
        storage_class: text(spec.and_then(|s| s.storage_class_name.as_deref()), "—"),
        bound_to,
        mounted_by: Vec::new(),
    }
}

/// Events about objects Kubeglass shows; other kinds return `None`.
pub fn event(e: &k8s::Event, now: Timestamp) -> Option<KubeEvent> {
    let name = SafeText::from(e.involved_object.name.as_deref()?);
    let object = match e.involved_object.kind.as_deref()? {
        "Pod" => ResourceRef::Pod(name),
        "Service" => ResourceRef::Service(name),
        "PersistentVolumeClaim" => ResourceRef::Pvc(name),
        "PersistentVolume" => ResourceRef::Pv(name),
        _ => return None,
    };
    let series = e.series.as_ref();
    let last_seen = series
        .and_then(|s| s.last_observed_time.as_ref())
        .map(|t| t.0)
        .or_else(|| e.last_timestamp.as_ref().map(|t| t.0))
        .or_else(|| e.event_time.as_ref().map(|t| t.0))
        .or_else(|| e.first_timestamp.as_ref().map(|t| t.0))
        .or_else(|| e.metadata.creation_timestamp.as_ref().map(|t| t.0));
    let count = series.and_then(|s| s.count).or(e.count).unwrap_or(1);
    Some(KubeEvent {
        object,
        event_type: if e.type_.as_deref() == Some("Warning") {
            EventType::Warning
        } else {
            EventType::Normal
        },
        reason: text(e.reason.as_deref(), ""),
        message: text(e.message.as_deref(), ""),
        age: age(last_seen, now),
        count: u32::try_from(count).unwrap_or(1),
    })
}

/// YAML for the viewer, without `metadata.managedFields` (noise for humans).
pub fn yaml_without_managed_fields<T: Serialize>(obj: &T) -> String {
    let mut value = match serde_json::to_value(obj) {
        Ok(v) => v,
        Err(e) => return format!("# cannot render YAML: {e}"),
    };
    if let Some(meta) = value.get_mut("metadata").and_then(|m| m.as_object_mut()) {
        meta.remove("managedFields");
    }
    serde_norway::to_string(&value).unwrap_or_else(|e| format!("# cannot render YAML: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EventType, PodStatus, ServiceType, StorageKind};
    use serde::de::DeserializeOwned;

    fn load<T: DeserializeOwned>(json: &str) -> Vec<T> {
        serde_json::from_str(json).expect("fixture parses")
    }

    fn now() -> Timestamp {
        "2026-11-06T12:00:00Z".parse().unwrap()
    }

    fn pods() -> Vec<k8s::Pod> {
        load(include_str!("../../tests/fixtures/pods.json"))
    }

    fn by_name<'a>(pods: &'a [Pod], name: &str) -> &'a Pod {
        pods.iter().find(|p| p.name.as_str() == name).unwrap()
    }

    #[test]
    fn running_pod_with_init_and_sidecar() {
        let all: Vec<Pod> = pods().iter().map(|p| pod(p, now())).collect();
        let p = by_name(&all, "api-1");
        assert_eq!(p.status, PodStatus::Running);
        assert_eq!(p.ready(), (1, 2));
        assert_eq!(p.restarts(), 3);
        assert_eq!(crate::model::format_age(p.age), "3h00m");
        assert_eq!(p.owner.as_ref().unwrap().as_str(), "ReplicaSet/api-7c9");
        assert_eq!(
            (p.node.as_str(), p.pod_ip.as_str()),
            ("node-1", "10.244.1.17")
        );
        assert_eq!(
            p.labels.get(&SafeText::from("app")).unwrap().as_str(),
            "api"
        );
        let names: Vec<_> = p
            .containers
            .iter()
            .map(|c| (c.name.as_str(), c.init))
            .collect();
        assert_eq!(
            names,
            [("init-perms", true), ("api", false), ("envoy", false)]
        );
        assert_eq!(
            p.containers[1].image.as_str(),
            "registry.example.test/api:v2"
        );
    }

    #[test]
    fn statuses_follow_kubectl() {
        let all: Vec<Pod> = pods().iter().map(|p| pod(p, now())).collect();
        let status = |n| by_name(&all, n).status;
        assert_eq!(status("billing-0"), PodStatus::CrashLoopBackOff);
        assert_eq!(status("sync-1"), PodStatus::Pending);
        assert_eq!(status("migrate-1"), PodStatus::Completed);
        assert_eq!(status("old-1"), PodStatus::Terminating);
        assert_eq!(status("web-1"), PodStatus::Failing);
        assert_eq!(by_name(&all, "web-1").status_label(), "ImagePullBackOff");
        assert_eq!(by_name(&all, "billing-0").restarts(), 17);
        let sync = by_name(&all, "sync-1");
        assert_eq!(
            sync.containers[0].waiting_reason.as_ref().unwrap().as_str(),
            "ContainerCreating"
        );
        assert_eq!(crate::model::format_age(sync.age), "2m");
    }

    #[test]
    fn escape_sequences_in_names_are_sanitized() {
        let all: Vec<Pod> = pods().iter().map(|p| pod(p, now())).collect();
        let p = all.last().unwrap();
        assert_eq!(p.name.as_str(), "evil-pod");
        assert_eq!(p.pod_ip.as_str(), "<none>");
        assert_eq!(p.ready(), (0, 1));
    }

    #[test]
    fn services_with_endpoint_slices() {
        let slices: Vec<EndpointSlice> =
            load(include_str!("../../tests/fixtures/endpointslices.json"));
        let svcs: Vec<Service> =
            load::<k8s::Service>(include_str!("../../tests/fixtures/services.json"))
                .iter()
                .map(|s| service(s, &slices))
                .collect();
        let api = &svcs[0];
        assert_eq!(api.service_type, ServiceType::ClusterIP);
        assert_eq!(
            crate::model::format_ports(&api.ports),
            "80→8080/TCP,9090→grpc/TCP"
        );
        assert_eq!(
            api.ready_endpoints, 3,
            "ready=true and ready unset count; ready=false does not"
        );
        assert_eq!(api.cluster_ip.as_str(), "10.96.14.22");
        let lb = &svcs[1];
        assert_eq!(lb.service_type, ServiceType::LoadBalancer);
        assert_eq!(crate::model::format_ports(&lb.ports), "443:31443/TCP");
        assert_eq!(lb.ready_endpoints, 0);
        let headless = &svcs[2];
        assert_eq!(headless.cluster_ip.as_str(), "None");
        assert_eq!(crate::model::format_ports(&headless.ports), "9100/TCP");
        assert!(headless.selector.is_empty());
    }

    #[test]
    fn claims_and_volumes() {
        let raw_pods = pods();
        let claims: Vec<StorageItem> =
            load::<k8s::PersistentVolumeClaim>(include_str!("../../tests/fixtures/pvcs.json"))
                .iter()
                .map(|c| pvc(c, &raw_pods))
                .collect();
        let vols: Vec<StorageItem> =
            load::<k8s::PersistentVolume>(include_str!("../../tests/fixtures/pvs.json"))
                .iter()
                .map(pv)
                .collect();
        let row = |s: &StorageItem| {
            (
                s.kind,
                s.name.to_string(),
                s.status.to_string(),
                s.capacity.to_string(),
                s.access_modes.to_string(),
                s.storage_class.to_string(),
                s.bound_to.as_ref().map(ToString::to_string),
                s.mounted_by
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
            )
        };
        assert_eq!(
            row(&claims[0]),
            (
                StorageKind::Pvc,
                "data-api".into(),
                "Bound".into(),
                "20Gi".into(),
                "RWO".into(),
                "longhorn".into(),
                Some("pvc-3e1a".into()),
                vec!["api-1".to_string()]
            )
        );
        assert_eq!(
            row(&claims[1]),
            (
                StorageKind::Pvc,
                "cache".into(),
                "Pending".into(),
                "5Gi".into(),
                "RWX,ROX".into(),
                "—".into(),
                None,
                vec![]
            )
        );
        assert_eq!(
            row(&vols[0]),
            (
                StorageKind::Pv,
                "pvc-3e1a".into(),
                "Bound".into(),
                "20Gi".into(),
                "RWOP".into(),
                "longhorn".into(),
                Some("shop/data-api".into()),
                vec![]
            )
        );
        assert_eq!(row(&vols[1]).2, "Released");
    }

    #[test]
    fn events_for_supported_kinds_only() {
        let events: Vec<KubeEvent> =
            load::<k8s::Event>(include_str!("../../tests/fixtures/events.json"))
                .iter()
                .filter_map(|e| event(e, now()))
                .collect();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].object, ResourceRef::Pod("billing-0".into()));
        assert_eq!(events[0].event_type, EventType::Warning);
        assert_eq!(
            (events[0].reason.as_str(), events[0].count),
            ("BackOff", 17)
        );
        assert_eq!(crate::model::format_age(events[0].age), "1m");
        assert_eq!(events[1].object, ResourceRef::Service("api".into()));
        assert_eq!(
            (events[1].count, crate::model::format_age(events[1].age)),
            (4, "10m".to_string())
        );
    }

    #[test]
    fn yaml_drops_managed_fields() {
        let p: k8s::Pod = serde_json::from_value(serde_json::json!({
            "metadata": {"name": "api-1", "managedFields": [{"manager": "kubectl"}]},
            "spec": {"containers": [{"name": "api", "image": "api:v2"}]}
        }))
        .unwrap();
        let y = yaml_without_managed_fields(&p);
        assert!(y.contains("name: api-1"), "{y}");
        assert!(!y.contains("managedFields"), "{y}");
    }

    #[test]
    fn future_timestamps_give_zero_age() {
        let p: k8s::Pod = serde_json::from_value(serde_json::json!({
            "metadata": {"name": "x", "creationTimestamp": "2026-11-06T12:05:00Z"}
        }))
        .unwrap();
        assert_eq!(pod(&p, now()).age, Duration::ZERO);
    }
}
