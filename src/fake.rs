//! Built-in sample cluster matching the design PDF. Used by the phase 1
//! prototype and by tests; replaced by real API data in phase 2.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::time::Duration;

use crate::model::{
    ClusterSnapshot, Container, EventType, KubeEvent, Pod, PodStatus, ResourceRef, Service,
    ServicePort, ServiceType, StorageItem, StorageKind,
};
use crate::text::SafeText;

const REG: &str = "registry.acme.io/payments";
const BILLING_DIGEST: &str =
    "sha256:9f2c4b7e1d0a3c5f6e7d8c9b0a1f2e3d4c5b6a7980f1e2d3c4b5a69788e81a";

/// Number of lines a running container has "already logged" when first opened.
const INITIAL_LINES: u64 = 34;
/// Simulated time between log lines, as in the design PDF.
const LINE_INTERVAL_MS: u64 = 437;

fn t(s: &str) -> SafeText {
    SafeText::from(s)
}

fn labels(app: &str) -> BTreeMap<SafeText, SafeText> {
    BTreeMap::from([(t("app"), t(app))])
}

fn container(name: &str, image: &str, ready: bool, restarts: u32) -> Container {
    Container {
        name: t(name),
        image: t(image),
        ready,
        restarts,
        init: false,
        waiting_reason: None,
    }
}

fn hm(h: u64, m: u64) -> Duration {
    Duration::from_secs(h * 3600 + m * 60)
}

fn pod(
    name: &str,
    app: &str,
    status: PodStatus,
    containers: Vec<Container>,
    age: Duration,
    ip: &str,
    owner: Option<&str>,
) -> Pod {
    Pod {
        name: t(name),
        namespace: t("payments"),
        status,
        labels: labels(app),
        containers,
        age,
        node: t("ip-10-0-12-34.ec2.internal"),
        pod_ip: t(ip),
        owner: owner.map(t),
    }
}

fn svc(
    name: &str,
    service_type: ServiceType,
    ip: &str,
    ports: Vec<ServicePort>,
    app: &str,
    eps: u32,
) -> Service {
    Service {
        name: t(name),
        namespace: t("payments"),
        service_type,
        cluster_ip: t(ip),
        ports,
        selector: labels(app),
        ready_endpoints: eps,
    }
}

fn port(port: u16, target_port: u16, node_port: Option<u16>) -> ServicePort {
    ServicePort {
        port,
        target_port: target_port.to_string().into(),
        node_port,
        protocol: t("TCP"),
    }
}

#[allow(clippy::too_many_arguments)]
fn storage(
    kind: StorageKind,
    name: &str,
    status: &str,
    cap: &str,
    mode: &str,
    class: &str,
    bound_to: Option<&str>,
    mounted_by: &[&str],
) -> StorageItem {
    StorageItem {
        kind,
        name: t(name),
        status: t(status),
        capacity: t(cap),
        access_modes: t(mode),
        storage_class: t(class),
        bound_to: bound_to.map(t),
        mounted_by: mounted_by.iter().map(|m| t(m)).collect(),
    }
}

fn event(
    object: ResourceRef,
    event_type: EventType,
    reason: &str,
    message: &str,
    age: Duration,
    count: u32,
) -> KubeEvent {
    KubeEvent {
        object,
        event_type,
        reason: t(reason),
        message: t(message),
        age,
        count,
    }
}

/// The sample `payments` namespace shown in the design PDF.
pub fn sample_snapshot() -> ClusterSnapshot {
    let api = |restarts| {
        vec![
            container("api", &format!("{REG}/api:v2.14.0"), true, restarts),
            container("envoy", "envoyproxy/envoy:v1.31.2", true, 0),
        ]
    };
    let mut billing_init = container("init-perms", "busybox:1.36", true, 0);
    billing_init.init = true;
    let mut billing = container(
        "billing",
        &format!("{REG}/billing@{BILLING_DIGEST}"),
        false,
        17,
    );
    billing.waiting_reason = Some(t("CrashLoopBackOff"));
    let mut ledger = container(
        "ledger-sync",
        &format!("{REG}/ledger-sync:v0.9.2"),
        false,
        0,
    );
    ledger.waiting_reason = Some(t("ContainerCreating"));
    let migrate = container("migrate", &format!("{REG}/migrate:v2.14.0"), false, 0);

    let pods = vec![
        pod(
            "api-7c9d8f-x2k4",
            "api",
            PodStatus::Running,
            api(0),
            hm(3, 12),
            "10.244.1.17",
            Some("ReplicaSet/api-7c9d8f"),
        ),
        pod(
            "api-7c9d8f-q8n1",
            "api",
            PodStatus::Running,
            api(1),
            hm(3, 12),
            "10.244.2.9",
            Some("ReplicaSet/api-7c9d8f"),
        ),
        pod(
            "worker-65b9c6-p2m8",
            "worker",
            PodStatus::Running,
            vec![container(
                "worker",
                &format!("{REG}/worker:v4.3.1"),
                true,
                0,
            )],
            hm(28, 0),
            "10.244.1.22",
            Some("ReplicaSet/worker-65b9c6"),
        ),
        pod(
            "billing-0",
            "billing",
            PodStatus::CrashLoopBackOff,
            vec![billing_init, billing],
            hm(2, 5),
            "10.244.3.4",
            Some("StatefulSet/billing"),
        ),
        pod(
            "migrate-28841-zt5",
            "migrate",
            PodStatus::Completed,
            vec![migrate],
            hm(0, 14),
            "10.244.2.31",
            Some("Job/migrate-28841"),
        ),
        pod(
            "ledger-sync-5f7d-h2c",
            "ledger-sync",
            PodStatus::Pending,
            vec![ledger],
            hm(0, 2),
            "<none>",
            Some("ReplicaSet/ledger-sync-5f7d"),
        ),
    ];

    let services = vec![
        svc(
            "api",
            ServiceType::ClusterIP,
            "10.96.14.22",
            vec![port(80, 8080, None)],
            "api",
            2,
        ),
        svc(
            "api-grpc",
            ServiceType::ClusterIP,
            "10.96.14.40",
            vec![port(9090, 9090, None)],
            "api",
            2,
        ),
        svc(
            "billing",
            ServiceType::ClusterIP,
            "10.96.31.7",
            vec![port(8443, 8443, None)],
            "billing",
            0,
        ),
        svc(
            "worker-metrics",
            ServiceType::ClusterIP,
            "None",
            vec![port(9100, 9100, None)],
            "worker",
            1,
        ),
        svc(
            "payments-lb",
            ServiceType::LoadBalancer,
            "10.96.2.18",
            vec![port(443, 443, Some(31443))],
            "api",
            2,
        ),
    ];

    let storage = vec![
        storage(
            StorageKind::Pvc,
            "data-billing-0",
            "Bound",
            "20Gi",
            "RWO",
            "gp3",
            Some("pvc-3e1a9c52-0c09"),
            &["billing-0"],
        ),
        storage(
            StorageKind::Pvc,
            "ledger-cache",
            "Pending",
            "5Gi",
            "RWO",
            "gp3",
            None,
            &["ledger-sync-5f7d-h2c"],
        ),
        storage(
            StorageKind::Pvc,
            "uploads",
            "Bound",
            "50Gi",
            "RWX",
            "efs",
            None,
            &[],
        ),
        storage(
            StorageKind::Pv,
            "pvc-3e1a9c52-0c09",
            "Bound",
            "20Gi",
            "RWO",
            "gp3",
            Some("payments/data-billing-0"),
            &[],
        ),
        storage(
            StorageKind::Pv,
            "pvc-77b2d4f1-141f",
            "Released",
            "50Gi",
            "RWO",
            "gp3",
            None,
            &[],
        ),
    ];

    let pod_ref = |n: &str| ResourceRef::Pod(t(n));
    let events = vec![
        event(
            pod_ref("billing-0"),
            EventType::Warning,
            "BackOff",
            "Back-off restarting failed container billing in pod billing-0",
            hm(0, 1),
            17,
        ),
        event(
            pod_ref("billing-0"),
            EventType::Normal,
            "Pulled",
            "Container image already present on machine",
            hm(0, 6),
            18,
        ),
        event(
            pod_ref("ledger-sync-5f7d-h2c"),
            EventType::Normal,
            "Scheduled",
            "Successfully assigned payments/ledger-sync-5f7d-h2c to ip-10-0-12-34.ec2.internal",
            hm(0, 2),
            1,
        ),
        event(
            pod_ref("ledger-sync-5f7d-h2c"),
            EventType::Warning,
            "FailedMount",
            "waiting for PVC ledger-cache to be bound",
            hm(0, 1),
            3,
        ),
        event(
            pod_ref("api-7c9d8f-q8n1"),
            EventType::Warning,
            "Unhealthy",
            "Readiness probe failed: HTTP probe failed with statuscode: 503",
            hm(1, 40),
            1,
        ),
        event(
            ResourceRef::Pvc(t("ledger-cache")),
            EventType::Normal,
            "WaitForFirstConsumer",
            "waiting for first consumer to be created before binding",
            hm(0, 2),
            4,
        ),
        event(
            ResourceRef::Service(t("billing")),
            EventType::Warning,
            "NoEndpoints",
            "no ready pods match selector app=billing",
            hm(0, 3),
            9,
        ),
    ];

    ClusterSnapshot {
        context: t("dev-east"),
        namespace: t("payments"),
        user: t("sso-readonly"),
        api_version: t("v1.31.4"),
        distro: t("eks"),
        pods,
        services,
        storage,
        events,
    }
}

/// Deterministic, PDF-matching log lines for sample pods.
#[derive(Debug, Default)]
pub struct FakeLogs {
    next_seq: HashMap<(String, String), u64>,
}

impl FakeLogs {
    pub fn new() -> Self {
        Self::default()
    }

    /// The last `tail` lines the container has "already" logged.
    pub fn initial(&self, pod: &Pod, container: &str, tail: usize) -> Vec<String> {
        let lines: Vec<String> = match pod.status {
            PodStatus::Pending | PodStatus::Failing => Vec::new(),
            PodStatus::CrashLoopBackOff | PodStatus::Failed => crash_lines(),
            PodStatus::Completed => migrate_lines(),
            PodStatus::Running | PodStatus::Terminating => (0..INITIAL_LINES)
                .map(|s| running_line(container, s))
                .collect(),
        };
        let skip = lines.len().saturating_sub(tail);
        lines.into_iter().skip(skip).collect()
    }

    /// The next streamed line, or `None` when the container is not producing logs.
    pub fn next_line(&mut self, pod: &Pod, container: &str) -> Option<String> {
        if pod.status != PodStatus::Running {
            return None;
        }
        let key = (pod.name.as_str().to_string(), container.to_string());
        let seq = self.next_seq.entry(key).or_insert(INITIAL_LINES);
        let line = running_line(container, *seq);
        *seq += 1;
        Some(line)
    }
}

fn timestamp(seq: u64) -> String {
    // 2026-11-06 12:04:11.000 + seq * 437ms
    let total_ms = (12 * 3600 + 4 * 60 + 11) * 1000 + seq * LINE_INTERVAL_MS;
    let (h, m, s, ms) = (
        (total_ms / 3_600_000) % 24,
        (total_ms / 60_000) % 60,
        (total_ms / 1000) % 60,
        total_ms % 1000,
    );
    format!("2026-11-06 {h:02}:{m:02}:{s:02}.{ms:03}")
}

fn fmt_line(seq: u64, level: &str, source: &str, msg: &str) -> String {
    format!("{} {level:<5} {source:<7} {msg}", timestamp(seq))
}

fn running_line(container: &str, seq: u64) -> String {
    let id = 8812 + seq;
    if container == "envoy" {
        let msg = match seq % 3 {
            0 => format!(
                "\"GET /v1/payments/{id} HTTP/1.1\" 200 upstream=api:8080 dur={}ms",
                9 + seq / 3
            ),
            1 => format!(
                "\"POST /v1/payments HTTP/1.1\" 201 upstream=api:8080 dur={}ms",
                16 + seq / 3
            ),
            _ => "\"GET /healthz HTTP/1.1\" 200 upstream=api:8080 dur=1ms".to_string(),
        };
        return fmt_line(seq, "INFO", "envoy", &msg);
    }
    if seq % 23 == 21 {
        return fmt_line(
            seq,
            "ERROR",
            "ledger",
            &format!(
                "upstream ledger-sync:7070 unavailable: context deadline exceeded (5s); payment pay_{id} queued for retry"
            ),
        );
    }
    match seq % 6 {
        0 => fmt_line(
            seq,
            "INFO",
            "http",
            &format!(
                "request completed method=GET path=/v1/payments/{id} status=200 dur={}ms",
                8 + seq / 3
            ),
        ),
        1 => fmt_line(
            seq,
            "INFO",
            "http",
            &format!(
                "request completed method=POST path=/v1/payments status=201 dur={}ms",
                15 + (seq - 1) / 3
            ),
        ),
        2 => fmt_line(seq, "DEBUG", "db", "pool stats open=12 idle=9 wait=0"),
        3 => fmt_line(
            seq,
            "INFO",
            "events",
            &format!("published payment.settled id=pay_{id} partition=3"),
        ),
        4 => fmt_line(
            seq,
            "INFO",
            "http",
            "request completed method=GET path=/healthz status=200 dur=1ms",
        ),
        _ => fmt_line(
            seq,
            "WARN",
            "ledger",
            "retrying connection to ledger-sync:7070 attempt=2 backoff=400ms",
        ),
    }
}

fn crash_lines() -> Vec<String> {
    [
        ("INFO", "main", "starting billing v2.14.0"),
        (
            "INFO",
            "config",
            "loaded config from /etc/billing/config.yaml",
        ),
        ("INFO", "db", "connecting to postgres billing-db:5432"),
        (
            "ERROR",
            "db",
            "connection refused: dial tcp 10.96.40.2:5432: connect: connection refused",
        ),
        (
            "ERROR",
            "main",
            "fatal: cannot start without database; exit code 1",
        ),
    ]
    .iter()
    .enumerate()
    .map(|(i, (l, s, m))| fmt_line(i as u64, l, s, m))
    .collect()
}

fn migrate_lines() -> Vec<String> {
    [
        ("INFO", "migrate", "connecting to postgres payments-db:5432"),
        ("INFO", "migrate", "current schema version 41"),
        ("INFO", "migrate", "applying 042_add_settlement_index.sql"),
        ("INFO", "migrate", "applying 043_ledger_outbox.sql"),
        ("INFO", "migrate", "schema version 43; done in 2.3s"),
    ]
    .iter()
    .enumerate()
    .map(|(i, (l, s, m))| fmt_line(i as u64, l, s, m))
    .collect()
}

/// A plausible manifest for the sample object. Phase 2 serializes real objects.
pub fn sample_yaml(r: &ResourceRef, snap: &ClusterSnapshot) -> String {
    let mut y = String::new();
    match r {
        ResourceRef::Pod(name) => {
            let Some(p) = snap.pods.iter().find(|p| &p.name == name) else {
                return String::new();
            };
            let _ = writeln!(
                y,
                "apiVersion: v1\nkind: Pod\nmetadata:\n  name: {}\n  namespace: {}\n  labels:",
                p.name, p.namespace
            );
            for (k, v) in &p.labels {
                let _ = writeln!(y, "    {k}: {v}");
            }
            let _ = writeln!(y, "spec:\n  nodeName: {}", p.node);
            let (init, regular): (Vec<_>, Vec<_>) = p.containers.iter().partition(|c| c.init);
            for (key, list) in [("initContainers", init), ("containers", regular)] {
                if list.is_empty() {
                    continue;
                }
                let _ = writeln!(y, "  {key}:");
                for c in list {
                    let _ = writeln!(y, "  - name: {}\n    image: {}", c.name, c.image);
                }
            }
            let _ = writeln!(
                y,
                "status:\n  phase: {}\n  podIP: {}",
                p.status.label(),
                p.pod_ip
            );
        }
        ResourceRef::Service(name) => {
            let Some(s) = snap.services.iter().find(|s| &s.name == name) else {
                return String::new();
            };
            let _ = writeln!(
                y,
                "apiVersion: v1\nkind: Service\nmetadata:\n  name: {}\n  namespace: {}\nspec:\n  type: {}\n  clusterIP: {}\n  selector:",
                s.name,
                s.namespace,
                s.service_type.label(),
                s.cluster_ip
            );
            for (k, v) in &s.selector {
                let _ = writeln!(y, "    {k}: {v}");
            }
            let _ = writeln!(y, "  ports:");
            for p in &s.ports {
                let _ = writeln!(
                    y,
                    "  - port: {}\n    targetPort: {}\n    protocol: {}",
                    p.port, p.target_port, p.protocol
                );
                if let Some(np) = p.node_port {
                    let _ = writeln!(y, "    nodePort: {np}");
                }
            }
        }
        ResourceRef::Pvc(name) | ResourceRef::Pv(name) => {
            let Some(s) = snap.storage.iter().find(|s| &s.name == name) else {
                return String::new();
            };
            let kind = if s.kind == StorageKind::Pvc {
                "PersistentVolumeClaim"
            } else {
                "PersistentVolume"
            };
            let _ = writeln!(
                y,
                "apiVersion: v1\nkind: {kind}\nmetadata:\n  name: {}",
                s.name
            );
            if s.kind == StorageKind::Pvc {
                let _ = writeln!(y, "  namespace: {}", snap.namespace);
            }
            let _ = writeln!(
                y,
                "spec:\n  accessModes: [{}]\n  storageClassName: {}\n  capacity:\n    storage: {}",
                s.access_modes, s.storage_class, s.capacity
            );
            if let Some(b) = &s.bound_to {
                let field = if s.kind == StorageKind::Pvc {
                    "volumeName"
                } else {
                    "claimRef"
                };
                let _ = writeln!(y, "  {field}: {b}");
            }
            let _ = writeln!(y, "status:\n  phase: {}", s.status);
        }
        ResourceRef::Image(_) => {}
    }
    y
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_logs_follow_pdf_timeline() {
        let s = sample_snapshot();
        let api = &s.pods[0];
        let first = FakeLogs::new().initial(api, "api", 500);
        assert_eq!(first.len(), 34);
        assert!(first[0].starts_with("2026-11-06 12:04:11.000 INFO"));
        assert!(first[1].starts_with("2026-11-06 12:04:11.437"));
        assert!(first[21].contains("ERROR") && first[21].contains("pay_8833"));
        assert!(first[33].starts_with("2026-11-06 12:04:25.421"));
    }

    #[test]
    fn pending_pod_has_no_logs() {
        let s = sample_snapshot();
        let p = &s.pods[5];
        assert!(FakeLogs::new().initial(p, "ledger-sync", 500).is_empty());
        assert!(FakeLogs::new().next_line(p, "ledger-sync").is_none());
    }

    #[test]
    fn streaming_continues_after_initial() {
        let s = sample_snapshot();
        let mut f = FakeLogs::new();
        let next = f.next_line(&s.pods[0], "api").unwrap();
        assert!(next.starts_with(&timestamp(34)));
    }

    #[test]
    fn tail_limits_initial() {
        let s = sample_snapshot();
        assert_eq!(FakeLogs::new().initial(&s.pods[0], "api", 10).len(), 10);
    }
}
