//! Domain types: the subset of Kubernetes objects Kubeglass displays.

use std::collections::BTreeMap;
use std::time::Duration;

use crate::text::{SafeText, shorten_digest};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    Pods,
    Images,
    Services,
    Storage,
}

impl ResourceKind {
    pub const ALL: [ResourceKind; 4] = [
        ResourceKind::Pods,
        ResourceKind::Images,
        ResourceKind::Services,
        ResourceKind::Storage,
    ];

    /// 1-based number shown in pane titles and used as the jump key.
    pub fn number(self) -> u8 {
        match self {
            ResourceKind::Pods => 1,
            ResourceKind::Images => 2,
            ResourceKind::Services => 3,
            ResourceKind::Storage => 4,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            ResourceKind::Pods => "Pods",
            ResourceKind::Images => "Images",
            ResourceKind::Services => "Services",
            ResourceKind::Storage => "Storage",
        }
    }

    pub fn from_number(n: u8) -> Option<Self> {
        ResourceKind::ALL
            .get(usize::from(n).checked_sub(1)?)
            .copied()
    }

    /// Index into per-kind arrays.
    pub fn index(self) -> usize {
        usize::from(self.number() - 1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PodStatus {
    Running,
    Pending,
    Completed,
    Failed,
    CrashLoopBackOff,
}

impl PodStatus {
    pub fn symbol(self) -> &'static str {
        match self {
            PodStatus::Running => "●",
            PodStatus::Pending => "◌",
            PodStatus::Completed => "✓",
            PodStatus::Failed | PodStatus::CrashLoopBackOff => "✕",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PodStatus::Running => "Running",
            PodStatus::Pending => "Pending",
            PodStatus::Completed => "Completed",
            PodStatus::Failed => "Failed",
            PodStatus::CrashLoopBackOff => "CrashLoopBackOff",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Container {
    pub name: SafeText,
    pub image: SafeText,
    pub ready: bool,
    pub restarts: u32,
    pub init: bool,
    pub waiting_reason: Option<SafeText>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pod {
    pub name: SafeText,
    pub namespace: SafeText,
    pub status: PodStatus,
    pub labels: BTreeMap<SafeText, SafeText>,
    pub containers: Vec<Container>,
    pub age: Duration,
    pub node: SafeText,
    pub pod_ip: SafeText,
    pub owner: Option<SafeText>,
}

impl Pod {
    /// (ready, total) over regular (non-init) containers, like `kubectl get pods`.
    pub fn ready(&self) -> (u32, u32) {
        let regular = self.containers.iter().filter(|c| !c.init);
        let (mut ready, mut total) = (0, 0);
        for c in regular {
            total += 1;
            if c.ready {
                ready += 1;
            }
        }
        (ready, total)
    }

    pub fn restarts(&self) -> u32 {
        self.containers
            .iter()
            .filter(|c| !c.init)
            .map(|c| c.restarts)
            .sum()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceType {
    ClusterIP,
    NodePort,
    LoadBalancer,
    ExternalName,
}

impl ServiceType {
    pub fn label(self) -> &'static str {
        match self {
            ServiceType::ClusterIP => "ClusterIP",
            ServiceType::NodePort => "NodePort",
            ServiceType::LoadBalancer => "LoadBalancer",
            ServiceType::ExternalName => "ExternalName",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ServicePort {
    pub port: u16,
    pub target_port: u16,
    pub node_port: Option<u16>,
    pub protocol: SafeText,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Service {
    pub name: SafeText,
    pub namespace: SafeText,
    pub service_type: ServiceType,
    pub cluster_ip: SafeText,
    pub ports: Vec<ServicePort>,
    pub selector: BTreeMap<SafeText, SafeText>,
    pub ready_endpoints: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageKind {
    Pvc,
    Pv,
}

impl StorageKind {
    pub fn label(self) -> &'static str {
        match self {
            StorageKind::Pvc => "PVC",
            StorageKind::Pv => "PV",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StorageItem {
    pub kind: StorageKind,
    pub name: SafeText,
    pub status: SafeText,
    pub capacity: SafeText,
    pub access_modes: SafeText,
    pub storage_class: SafeText,
    pub bound_to: Option<SafeText>,
    pub mounted_by: Vec<SafeText>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageRef {
    pub repository: SafeText,
    pub tag: Option<SafeText>,
    pub digest: Option<SafeText>,
}

impl ImageRef {
    /// Split `repo[:tag][@digest]`. A `:` only starts a tag when it comes after
    /// the last `/`, so registry ports (`host:5000/repo`) are kept in the repository.
    pub fn parse(raw: &SafeText) -> ImageRef {
        let raw = raw.as_str();
        let (name, digest) = match raw.split_once('@') {
            Some((n, d)) => (n, Some(SafeText::from(d))),
            None => (raw, None),
        };
        let last_slash = name.rfind('/').map_or(0, |i| i + 1);
        let (repository, tag) = match name[last_slash..].rfind(':') {
            Some(i) => {
                let split = last_slash + i;
                (
                    SafeText::from(&name[..split]),
                    Some(SafeText::from(&name[split + 1..])),
                )
            }
            None => (SafeText::from(name), None),
        };
        ImageRef {
            repository,
            tag,
            digest,
        }
    }

    pub fn tag_or_digest(&self) -> String {
        match (&self.tag, &self.digest) {
            (Some(tag), _) => tag.to_string(),
            (None, Some(digest)) => format!("@{}", shorten_digest(digest.as_str())),
            (None, None) => "latest".to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageUsage {
    pub raw: SafeText,
    pub image: ImageRef,
    /// (pod name, container name)
    pub users: Vec<(SafeText, SafeText)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventType {
    Normal,
    Warning,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KubeEvent {
    pub object: ResourceRef,
    pub event_type: EventType,
    pub reason: SafeText,
    pub message: SafeText,
    pub age: Duration,
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResourceRef {
    Pod(SafeText),
    Image(SafeText),
    Service(SafeText),
    Pvc(SafeText),
    Pv(SafeText),
}

impl ResourceRef {
    pub fn kind_label(&self) -> &'static str {
        match self {
            ResourceRef::Pod(_) => "Pod",
            ResourceRef::Image(_) => "Image",
            ResourceRef::Service(_) => "Service",
            ResourceRef::Pvc(_) => "PVC",
            ResourceRef::Pv(_) => "PV",
        }
    }

    pub fn name(&self) -> &SafeText {
        match self {
            ResourceRef::Pod(n)
            | ResourceRef::Image(n)
            | ResourceRef::Service(n)
            | ResourceRef::Pvc(n)
            | ResourceRef::Pv(n) => n,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClusterSnapshot {
    pub context: SafeText,
    pub namespace: SafeText,
    pub user: SafeText,
    pub api_version: SafeText,
    pub distro: SafeText,
    pub pods: Vec<Pod>,
    pub services: Vec<Service>,
    pub storage: Vec<StorageItem>,
    pub events: Vec<KubeEvent>,
}

/// Compact age as in the design: `42s`, `14m`, `2h05m`, `1d4h`.
pub fn format_age(d: Duration) -> String {
    let s = d.as_secs();
    let (days, hours, mins) = (s / 86_400, (s % 86_400) / 3600, (s % 3600) / 60);
    if days > 0 {
        format!("{days}d{hours}h")
    } else if hours > 0 {
        format!("{hours}h{mins:02}m")
    } else if mins > 0 {
        format!("{mins}m")
    } else {
        format!("{s}s")
    }
}

/// Port summary like the design: `80→8080/TCP`, `9090/TCP`, `443:31443/TCP`.
pub fn format_ports(ports: &[ServicePort]) -> String {
    ports
        .iter()
        .map(|p| {
            let head = match p.node_port {
                Some(np) => format!("{}:{np}", p.port),
                None if p.target_port != p.port => format!("{}→{}", p.port, p.target_port),
                None => p.port.to_string(),
            };
            format!("{head}/{}", p.protocol)
        })
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::sample_snapshot;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    fn port(port: u16, target_port: u16, node_port: Option<u16>) -> ServicePort {
        ServicePort {
            port,
            target_port,
            node_port,
            protocol: "TCP".into(),
        }
    }

    #[test]
    fn ages() {
        assert_eq!(format_age(secs(3 * 3600 + 12 * 60)), "3h12m");
        assert_eq!(format_age(secs(2 * 3600 + 5 * 60)), "2h05m");
        assert_eq!(format_age(secs(28 * 3600)), "1d4h");
        assert_eq!(format_age(secs(14 * 60)), "14m");
        assert_eq!(format_age(secs(42)), "42s");
    }

    #[test]
    fn ports() {
        assert_eq!(format_ports(&[port(80, 8080, None)]), "80→8080/TCP");
        assert_eq!(format_ports(&[port(9090, 9090, None)]), "9090/TCP");
        assert_eq!(
            format_ports(&[port(443, 443, Some(31443))]),
            "443:31443/TCP"
        );
        assert_eq!(
            format_ports(&[port(80, 80, None), port(443, 443, None)]),
            "80/TCP,443/TCP"
        );
    }

    #[test]
    fn image_refs() {
        let r = ImageRef::parse(&"registry.acme.io:5000/payments/api:v2.14.0".into());
        assert_eq!(
            (r.repository.as_str(), r.tag.as_ref().map(SafeText::as_str)),
            ("registry.acme.io:5000/payments/api", Some("v2.14.0"))
        );
        assert_eq!(ImageRef::parse(&"busybox".into()).tag_or_digest(), "latest");
        assert_eq!(
            ImageRef::parse(&"registry.acme.io/payments/billing@sha256:9f2c4b7e1d0a3c5f6e7d8c9b0a1f2e3d4c5b6a7980f1e2d3c4b5a69788e81a".into())
                .tag_or_digest(),
            "@sha256:9f2c…e81a"
        );
        assert_eq!(
            ImageRef::parse(&"registry.acme.io:5000/payments/api".into()).tag,
            None
        );
    }

    #[test]
    fn cluster_strings_in_collections_are_safe_text() {
        let r = ImageRef::parse(&SafeText::from("evil\x1b[2J/app:v1"));
        assert_eq!(r.repository.as_str(), "evil/app");
        let s = sample_snapshot();
        let (k, v): (&SafeText, &SafeText) = s.pods[0].labels.iter().next().unwrap();
        assert_eq!((k.as_str(), v.as_str()), ("app", "api"));
    }

    #[test]
    fn pod_ready_ignores_init() {
        let s = sample_snapshot();
        let b = s
            .pods
            .iter()
            .find(|p| p.name.as_str() == "billing-0")
            .unwrap();
        assert_eq!(b.ready(), (0, 1));
        assert_eq!(b.restarts(), 17);
    }

    #[test]
    fn kind_numbers_roundtrip() {
        for k in ResourceKind::ALL {
            assert_eq!(ResourceKind::from_number(k.number()), Some(k));
        }
        assert_eq!(ResourceKind::from_number(0), None);
        assert_eq!(ResourceKind::from_number(5), None);
    }
}
