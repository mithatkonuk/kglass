//! Read-only API calls with timeouts and user-facing error mapping.
//!
//! Every call is bounded by [`REQUEST_TIMEOUT`]; each list is fetched independently
//! so one failure (403, timeout) never hides the others.

use std::fmt::Debug;
use std::time::Duration;

use futures::io::{AsyncBufRead, AsyncBufReadExt};
use futures::{Stream, StreamExt, stream};
use k8s_openapi::api::core::v1 as k8s;
use kube::api::{ListParams, LogParams};
use kube::{Api, Client};
use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;
use tokio::time::timeout;

use super::config::describe_auth;
use super::convert::{self, EndpointSlice, Timestamp};
use crate::model::{KubeEvent, Pod, ResourceKind, ResourceRef, Service, StorageItem};
use crate::text::SafeText;

/// One Kubernetes permission kglass uses. `docs/rbac.md` is checked against
/// this list by `tests/rbac_doc.rs`.
pub struct Permission {
    /// API group (`""` is the core group).
    pub group: &'static str,
    pub resource: &'static str,
    pub verbs: &'static [&'static str],
    /// Granted by a ClusterRole (cluster-scoped resource) rather than a Role.
    pub cluster_scoped: bool,
    /// kglass works without it; the affected part shows a "Forbidden" note.
    pub optional: bool,
}

const fn perm(
    group: &'static str,
    resource: &'static str,
    verbs: &'static [&'static str],
    cluster_scoped: bool,
    optional: bool,
) -> Permission {
    Permission {
        group,
        resource,
        verbs,
        cluster_scoped,
        optional,
    }
}

/// Every read kglass performs. All read-only; Secrets are never read.
pub const PERMISSIONS: &[Permission] = &[
    perm("", "pods", &["get", "list"], false, false),
    perm("", "pods/log", &["get"], false, false),
    perm("", "services", &["get", "list"], false, false),
    perm(
        "discovery.k8s.io",
        "endpointslices",
        &["list"],
        false,
        false,
    ),
    perm("", "persistentvolumeclaims", &["get", "list"], false, false),
    perm("", "events", &["list"], false, true),
    perm("", "persistentvolumes", &["get", "list"], true, true),
    perm("", "namespaces", &["list"], true, true),
];

/// Upper bound for any single request (design D5).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// Items requested per page.
const PAGE_SIZE: u32 = 500;
/// Stop paging after this many items (large clusters; see design D13).
pub const MAX_ITEMS: usize = 5000;
/// The paging cap actually used; tiny in unit tests so truncation is testable.
const PAGE_CAP: usize = if cfg!(test) { 3 } else { MAX_ITEMS };
/// Longest log line kept (bytes); the rest is dropped and marked. With the
/// 5000-line buffer this bounds log memory to about 40 MiB.
pub const MAX_LOG_LINE: usize = 8 * 1024;
/// Appended to a line that was cut at [`MAX_LOG_LINE`].
pub const TRUNCATED_MARKER: &str = " …[line truncated]";

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum FetchError {
    #[error("Unauthorized (401): {0}")]
    Unauthorized(SafeText),
    #[error("Forbidden (403): cannot {verb} {resource} at the {scope} scope")]
    Forbidden {
        verb: &'static str,
        resource: &'static str,
        scope: SafeText,
        message: SafeText,
    },
    #[error("timed out after {}s", REQUEST_TIMEOUT.as_secs())]
    Timeout,
    #[error("cannot reach the cluster: {0}")]
    Unreachable(SafeText),
    #[error("API error {code}: {message}")]
    Api { code: u16, message: SafeText },
}

/// What a request was trying to do, for error messages.
#[derive(Clone, Copy)]
struct Action<'a> {
    verb: &'static str,
    resource: &'static str,
    scope: &'a str,
}

fn map_error(err: kube::Error, action: Action<'_>) -> FetchError {
    match err {
        kube::Error::Api(status) => match status.code {
            401 => FetchError::Unauthorized(SafeText::from(status.message.as_str())),
            403 => FetchError::Forbidden {
                verb: action.verb,
                resource: action.resource,
                scope: SafeText::from(action.scope),
                message: SafeText::from(status.message.as_str()),
            },
            code => FetchError::Api {
                code,
                message: SafeText::from(status.message.as_str()),
            },
        },
        kube::Error::Auth(e) => FetchError::Unauthorized(SafeText::from(describe_auth(&e))),
        other => FetchError::Unreachable(SafeText::from(other.to_string())),
    }
}

async fn bounded<T>(
    action: Action<'_>,
    fut: impl Future<Output = Result<T, kube::Error>>,
) -> Result<T, FetchError> {
    match timeout(REQUEST_TIMEOUT, fut).await {
        Err(_) => Err(FetchError::Timeout),
        Ok(result) => result.map_err(|e| map_error(e, action)),
    }
}

/// Items of one list call, and whether paging stopped at the cap.
struct Listed<K> {
    items: Vec<K>,
    truncated: bool,
}

/// List all items (paged), stopping at [`MAX_ITEMS`].
async fn list<K>(api: Api<K>, action: Action<'_>) -> Result<Listed<K>, FetchError>
where
    K: Clone + DeserializeOwned + Debug,
{
    let mut items = Vec::new();
    let mut params = ListParams::default().limit(PAGE_SIZE);
    loop {
        let page = bounded(action, api.list(&params)).await?;
        items.extend(page.items);
        match page.metadata.continue_ {
            Some(token) if !token.is_empty() => {
                if items.len() >= PAGE_CAP {
                    return Ok(Listed {
                        items,
                        truncated: true,
                    });
                }
                params = params.continue_token(&token);
            }
            _ => {
                return Ok(Listed {
                    items,
                    truncated: false,
                });
            }
        }
    }
}

fn was_truncated<K>(listed: &Result<Listed<K>, FetchError>) -> bool {
    listed.as_ref().is_ok_and(|l| l.truncated)
}

/// Everything the four lists need for one namespace.
#[derive(Clone, Debug, PartialEq)]
pub struct Lists {
    pub pods: Result<Vec<Pod>, FetchError>,
    pub services: Result<Vec<Service>, FetchError>,
    /// PVCs, then the PVs claimed from this namespace. Fails only when PVCs cannot be listed.
    pub storage: Result<Vec<StorageItem>, FetchError>,
    /// Set when PVCs listed but PVs (cluster scope) did not.
    pub storage_partial: Option<FetchError>,
    pub events: Result<Vec<KubeEvent>, FetchError>,
    /// Lists cut off at [`MAX_ITEMS`]; Images follow Pods.
    pub truncated: Vec<ResourceKind>,
}

/// Which namespaces to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    Namespace(String),
    /// `-A`: every namespace. Namespaced objects are keyed `namespace/name`.
    All,
}

impl Scope {
    /// Namespace and object name for an API call. Under `All`, keys are
    /// `namespace/name` (Kubernetes names cannot contain `/`).
    pub fn split(&self, key: &str) -> (String, String) {
        match self {
            Scope::Namespace(ns) => (ns.clone(), key.to_string()),
            Scope::All => match key.split_once('/') {
                Some((ns, name)) => (ns.to_string(), name.to_string()),
                None => (String::new(), key.to_string()),
            },
        }
    }

    /// How a list call's scope is described in errors.
    fn describe(&self) -> String {
        match self {
            Scope::Namespace(ns) => format!("namespace \"{ns}\""),
            Scope::All => "cluster".to_string(),
        }
    }

    fn api<K>(&self, client: &Client) -> Api<K>
    where
        K: kube::Resource<Scope = kube::core::NamespaceResourceScope, DynamicType = ()>,
    {
        match self {
            Scope::Namespace(ns) => Api::namespaced(client.clone(), ns),
            Scope::All => Api::all(client.clone()),
        }
    }
}

/// `namespace/name`, the key of a namespaced object under `-A`.
fn qualified(namespace: Option<&str>, name: &SafeText) -> SafeText {
    SafeText::from(format!("{}/{name}", namespace.unwrap_or_default()))
}

pub async fn fetch_lists(client: &Client, scope: &Scope, now: Timestamp) -> Lists {
    let described = scope.describe();
    let in_scope = |resource| Action {
        verb: "list",
        resource,
        scope: &described,
    };
    let cluster = |resource| Action {
        verb: "list",
        resource,
        scope: "cluster",
    };
    let (pods, services, slices, pvcs, pvs, events) = tokio::join!(
        list(scope.api::<k8s::Pod>(client), in_scope("pods")),
        list(scope.api::<k8s::Service>(client), in_scope("services")),
        list(
            scope.api::<EndpointSlice>(client),
            in_scope("endpointslices")
        ),
        list(
            scope.api::<k8s::PersistentVolumeClaim>(client),
            in_scope("persistentvolumeclaims")
        ),
        list(
            Api::<k8s::PersistentVolume>::all(client.clone()),
            cluster("persistentvolumes")
        ),
        list(scope.api::<k8s::Event>(client), in_scope("events")),
    );
    let mut truncated = Vec::new();
    if was_truncated(&pods) {
        truncated.extend([ResourceKind::Pods, ResourceKind::Images]);
    }
    if was_truncated(&services) {
        truncated.push(ResourceKind::Services);
    }
    if was_truncated(&pvcs) || was_truncated(&pvs) {
        truncated.push(ResourceKind::Storage);
    }
    let all = *scope == Scope::All;
    let pods = pods.map(|l| l.items);
    let services = services.map(|l| l.items);
    let slices = slices.map(|l| l.items);
    let pvcs = pvcs.map(|l| l.items);
    let pvs = pvs.map(|l| l.items);
    let events = events.map(|l| l.items);
    let raw_pods = pods.as_deref().unwrap_or_default();

    let services = services.and_then(|svcs| {
        let slices = slices?;
        Ok(svcs
            .iter()
            .map(|s| {
                let mut svc = convert::service(s, &slices);
                if all {
                    svc.name = qualified(s.metadata.namespace.as_deref(), &svc.name);
                }
                svc
            })
            .collect())
    });
    let (storage, storage_partial) = match (pvcs, pvs) {
        (Err(e), _) => (Err(e), None),
        (Ok(claims), volumes) => {
            let mut items: Vec<StorageItem> = claims
                .iter()
                .map(|c| {
                    let mut item = convert::pvc(c, raw_pods);
                    if all {
                        let ns = c.metadata.namespace.as_deref();
                        item.name = qualified(ns, &item.name);
                        item.mounted_by =
                            item.mounted_by.iter().map(|p| qualified(ns, p)).collect();
                    }
                    item
                })
                .collect();
            let partial = match volumes {
                Ok(vs) => {
                    // PVs are cluster-scoped; in one namespace, show only those it claims.
                    items.extend(
                        vs.iter()
                            .filter(|v| match scope {
                                Scope::Namespace(ns) => claim_namespace(v) == Some(ns.as_str()),
                                Scope::All => true,
                            })
                            .map(convert::pv),
                    );
                    None
                }
                Err(e) => Some(e),
            };
            (Ok(items), partial)
        }
    };
    let pods = pods.map(|ps| {
        ps.iter()
            .map(|p| {
                let mut pod = convert::pod(p, now);
                if all {
                    pod.name = qualified(p.metadata.namespace.as_deref(), &pod.name);
                }
                pod
            })
            .collect()
    });
    let events = events.map(|es| {
        es.iter()
            .filter_map(|e| {
                let mut event = convert::event(e, now)?;
                if all {
                    let ns = e
                        .involved_object
                        .namespace
                        .as_deref()
                        .or(e.metadata.namespace.as_deref());
                    event.object = match event.object {
                        ResourceRef::Pod(n) => ResourceRef::Pod(qualified(ns, &n)),
                        ResourceRef::Service(n) => ResourceRef::Service(qualified(ns, &n)),
                        ResourceRef::Pvc(n) => ResourceRef::Pvc(qualified(ns, &n)),
                        other => other,
                    };
                }
                Some(event)
            })
            .collect()
    });
    Lists {
        pods,
        services,
        storage,
        storage_partial,
        events,
        truncated,
    }
}

fn claim_namespace(v: &k8s::PersistentVolume) -> Option<&str> {
    v.spec
        .as_ref()
        .and_then(|s| s.claim_ref.as_ref())
        .and_then(|r| r.namespace.as_deref())
}

async fn yaml_of<K>(api: Api<K>, name: &str, action: Action<'_>) -> Result<String, FetchError>
where
    K: Clone + DeserializeOwned + Debug + Serialize,
{
    let obj = bounded(action, api.get(name)).await?;
    Ok(convert::yaml_without_managed_fields(&obj))
}

/// YAML of the selected object (images have none and return an empty string).
pub async fn fetch_yaml(
    client: &Client,
    scope: &Scope,
    target: &ResourceRef,
) -> Result<String, FetchError> {
    let (ns, name) = scope.split(target.name().as_str());
    let ns = ns.as_str();
    let name = name.as_str();
    let described = format!("namespace \"{ns}\"");
    let get = |resource| Action {
        verb: "get",
        resource,
        scope: &described,
    };
    match target {
        ResourceRef::Pod(_) => {
            yaml_of(
                Api::<k8s::Pod>::namespaced(client.clone(), ns),
                name,
                get("pods"),
            )
            .await
        }
        ResourceRef::Service(_) => {
            yaml_of(
                Api::<k8s::Service>::namespaced(client.clone(), ns),
                name,
                get("services"),
            )
            .await
        }
        ResourceRef::Pvc(_) => {
            yaml_of(
                Api::<k8s::PersistentVolumeClaim>::namespaced(client.clone(), ns),
                name,
                get("persistentvolumeclaims"),
            )
            .await
        }
        ResourceRef::Pv(_) => {
            let action = Action {
                verb: "get",
                resource: "persistentvolumes",
                scope: "cluster",
            };
            yaml_of(
                Api::<k8s::PersistentVolume>::all(client.clone()),
                name,
                action,
            )
            .await
        }
        ResourceRef::Image(_) => Ok(String::new()),
    }
}

pub async fn server_version(client: &Client) -> Result<SafeText, FetchError> {
    let action = Action {
        verb: "get",
        resource: "version",
        scope: "cluster",
    };
    let info = bounded(action, client.apiserver_version()).await?;
    Ok(SafeText::from(info.git_version.as_str()))
}

pub async fn namespaces(client: &Client) -> Result<Vec<SafeText>, FetchError> {
    let action = Action {
        verb: "list",
        resource: "namespaces",
        scope: "cluster",
    };
    let all = list(Api::<k8s::Namespace>::all(client.clone()), action)
        .await?
        .items;
    Ok(all
        .iter()
        .filter_map(|n| n.metadata.name.as_deref().map(SafeText::from))
        .collect())
}

/// Follow a container's log from the last `tail` lines. Lines are decoded lossily,
/// sanitized, and capped at [`MAX_LOG_LINE`] bytes.
pub fn log_lines(
    client: Client,
    ns: String,
    pod: String,
    container: String,
    tail: i64,
    previous: bool,
) -> impl Stream<Item = Result<SafeText, FetchError>> + Send {
    let open = async move {
        let api = Api::<k8s::Pod>::namespaced(client, &ns);
        let params = log_params(container, tail, previous);
        let scope = format!("namespace \"{ns}\"");
        let action = Action {
            verb: "get",
            resource: "pods/log",
            scope: &scope,
        };
        bounded(action, api.log_stream(&pod, &params))
            .await
            .map(Box::pin)
    };
    stream::once(open).flat_map(|opened| match opened {
        Ok(reader) => lines_lossy(reader).left_stream(),
        Err(e) => stream::once(async { Err(e) }).right_stream(),
    })
}

/// Log request parameters. Previous-instance logs are complete, so they are not followed.
pub fn log_params(container: String, tail: i64, previous: bool) -> LogParams {
    LogParams {
        container: Some(container),
        follow: !previous,
        previous,
        tail_lines: Some(tail),
        ..LogParams::default()
    }
}

/// Split a byte stream into sanitized lines of at most [`MAX_LOG_LINE`] bytes.
pub(crate) fn lines_lossy<R>(reader: R) -> impl Stream<Item = Result<SafeText, FetchError>> + Send
where
    R: AsyncBufRead + Unpin + Send,
{
    stream::try_unfold(reader, |mut reader| async move {
        let mut line = Vec::new();
        let mut saw_any = false;
        let mut truncated = false;
        loop {
            let chunk = reader
                .fill_buf()
                .await
                .map_err(|e| FetchError::Unreachable(SafeText::from(e.to_string())))?;
            if chunk.is_empty() {
                break;
            }
            saw_any = true;
            let (take, done) = match chunk.iter().position(|b| *b == b'\n') {
                Some(pos) => (pos, true),
                None => (chunk.len(), false),
            };
            let room = MAX_LOG_LINE.saturating_sub(line.len());
            truncated |= take > room;
            line.extend_from_slice(&chunk[..take.min(room)]);
            reader.consume_unpin(take + usize::from(done));
            if done {
                break;
            }
        }
        if !saw_any {
            return Ok(None);
        }
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        let mut text = String::from_utf8_lossy(&line).into_owned();
        if truncated {
            text.push_str(TRUNCATED_MARKER);
        }
        let text = SafeText::from(text);
        Ok(Some((text, reader)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::testing::{NS, Reply, healthy, mock, status_json};
    use futures::StreamExt;

    fn now() -> Timestamp {
        "2026-11-06T12:00:00Z".parse().unwrap()
    }

    #[tokio::test]
    async fn lists_fetched_and_converted() {
        let lists = fetch_lists(&mock(healthy), &Scope::Namespace(NS.into()), now()).await;
        assert_eq!(lists.pods.as_ref().unwrap().len(), 7);
        let services = lists.services.as_ref().unwrap();
        assert_eq!(services[0].ready_endpoints, 3);
        let storage = lists.storage.as_ref().unwrap();
        let names: Vec<&str> = storage.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            ["data-api", "cache", "pvc-3e1a", "pvc-77b2"],
            "PVs claimed from other namespaces are not shown"
        );
        assert_eq!(storage[0].mounted_by[0].as_str(), "api-1");
        assert!(lists.storage_partial.is_none());
        assert_eq!(lists.events.as_ref().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn forbidden_pv_keeps_pvcs() {
        fn route(path: &str) -> Reply {
            if path == "/api/v1/persistentvolumes" {
                return Some((
                    403,
                    status_json(403, "Forbidden", "persistentvolumes is forbidden"),
                ));
            }
            healthy(path)
        }
        let lists = fetch_lists(&mock(route), &Scope::Namespace(NS.into()), now()).await;
        let storage = lists.storage.unwrap();
        assert_eq!(storage.len(), 2, "PVCs still shown");
        assert_eq!(
            lists.storage_partial,
            Some(FetchError::Forbidden {
                verb: "list",
                resource: "persistentvolumes",
                scope: "cluster".into(),
                message: "persistentvolumes is forbidden".into(),
            })
        );
        assert!(lists.pods.is_ok());
    }

    #[tokio::test]
    async fn unauthorized_maps_to_unauthorized() {
        fn route(_: &str) -> Reply {
            Some((401, status_json(401, "Unauthorized", "Unauthorized")))
        }
        let lists = fetch_lists(&mock(route), &Scope::Namespace(NS.into()), now()).await;
        assert_eq!(
            lists.pods,
            Err(FetchError::Unauthorized("Unauthorized".into()))
        );
        assert!(server_version(&mock(route)).await.is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_maps_to_timeout() {
        fn route(_: &str) -> Reply {
            None
        }
        let lists = fetch_lists(&mock(route), &Scope::Namespace(NS.into()), now()).await;
        assert_eq!(lists.pods, Err(FetchError::Timeout));
        assert_eq!(lists.storage, Err(FetchError::Timeout));
    }

    #[tokio::test]
    async fn log_lines_sanitized_and_lossy() {
        let lines: Vec<_> = log_lines(
            mock(healthy),
            NS.into(),
            "api-1".into(),
            "api".into(),
            500,
            false,
        )
        .collect()
        .await;
        let lines: Vec<String> = lines.into_iter().map(|l| l.unwrap().to_string()).collect();
        assert_eq!(lines, ["line one", "bad \u{fffd} byte", "escape done"]);
    }

    #[tokio::test]
    async fn very_long_log_line_is_capped_with_marker() {
        let reader =
            futures::io::Cursor::new([vec![b'x'; MAX_LOG_LINE * 2], b"\nnext\n".to_vec()].concat());
        let lines: Vec<_> = lines_lossy(reader).collect().await;
        let first = lines[0].as_ref().unwrap().as_str();
        assert_eq!(MAX_LOG_LINE, 8 * 1024);
        assert!(
            first.starts_with("xxxx") && first.ends_with(TRUNCATED_MARKER),
            "{}",
            &first[first.len() - 30..]
        );
        assert_eq!(first.len(), MAX_LOG_LINE + TRUNCATED_MARKER.len());
        assert_eq!(lines[1].as_ref().unwrap().as_str(), "next");
    }

    #[tokio::test]
    async fn yaml_version_and_namespaces() {
        let client = mock(healthy);
        let y = fetch_yaml(
            &client,
            &Scope::Namespace(NS.into()),
            &ResourceRef::Pod("api-1".into()),
        )
        .await
        .unwrap();
        assert!(
            y.contains("name: api-1") && !y.contains("managedFields"),
            "{y}"
        );
        assert_eq!(
            server_version(&client).await.unwrap().as_str(),
            "v1.36.4+rke2r1"
        );
        let names: Vec<String> = namespaces(&client)
            .await
            .unwrap()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(names, ["default", "shop"]);
    }

    #[test]
    fn auth_errors_hide_plugin_output() {
        let err = kube::Error::Auth(kube::client::AuthError::AuthExec("token SECRET".into()));
        let action = Action {
            verb: "list",
            resource: "pods",
            scope: "cluster",
        };
        match map_error(err, action) {
            FetchError::Unauthorized(msg) => assert!(!msg.as_str().contains("SECRET"), "{msg}"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn paging_stops_at_cap_and_reports_truncation() {
        fn route(path: &str) -> Reply {
            if path == "/api/v1/namespaces/shop/pods" {
                let item = r#"{"metadata":{"name":"p"},"spec":{"containers":[]}}"#;
                let body = format!(
                    r#"{{"apiVersion":"v1","kind":"PodList","metadata":{{"continue":"next"}},"items":[{item},{item}]}}"#
                );
                return Some((200, body.into_bytes()));
            }
            healthy(path)
        }
        let lists = fetch_lists(&mock(route), &Scope::Namespace(NS.into()), now()).await;
        assert_eq!(
            lists.pods.unwrap().len(),
            4,
            "two pages of two reach the test cap of 3"
        );
        assert_eq!(lists.truncated, [ResourceKind::Pods, ResourceKind::Images]);
        let healthy_lists = fetch_lists(&mock(healthy), &Scope::Namespace(NS.into()), now()).await;
        assert!(healthy_lists.truncated.is_empty());
    }

    #[tokio::test]
    async fn all_namespaces_lists_cluster_wide_and_qualifies_names() {
        use crate::cluster::testing::all_namespaces;
        let lists = fetch_lists(&mock(all_namespaces), &Scope::All, now()).await;
        let pods: Vec<String> = lists
            .pods
            .unwrap()
            .iter()
            .map(|p| p.name.to_string())
            .collect();
        assert_eq!(pods[0], "shop/api-1");
        let services = lists.services.unwrap();
        assert_eq!(
            (services[0].name.as_str(), services[0].ready_endpoints),
            ("shop/api", 3)
        );
        let storage = lists.storage.unwrap();
        let names: Vec<&str> = storage.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "shop/data-api",
                "shop/cache",
                "pvc-3e1a",
                "pvc-77b2",
                "pvc-other"
            ]
        );
        assert_eq!(storage[0].mounted_by[0].as_str(), "shop/api-1");
        let events = lists.events.unwrap();
        assert_eq!(events[0].object, ResourceRef::Pod("shop/billing-0".into()));
    }

    #[test]
    fn keys_split_into_namespace_and_name() {
        let all = Scope::All;
        assert_eq!(
            all.split("shop/api-1"),
            ("shop".to_string(), "api-1".to_string())
        );
        let one = Scope::Namespace("team-a".into());
        assert_eq!(
            one.split("api-1"),
            ("team-a".to_string(), "api-1".to_string())
        );
    }

    #[test]
    fn previous_logs_do_not_follow() {
        let live = log_params("api".into(), 500, false);
        assert!(live.follow && !live.previous);
        let prev = log_params("api".into(), 500, true);
        assert!(!prev.follow && prev.previous);
        assert_eq!(
            (prev.tail_lines, prev.container.as_deref()),
            (Some(500), Some("api"))
        );
    }

    #[tokio::test]
    async fn missing_object_maps_to_api_error() {
        let err = fetch_yaml(
            &mock(healthy),
            &Scope::Namespace(NS.into()),
            &ResourceRef::Service("gone".into()),
        )
        .await
        .unwrap_err();
        assert_eq!(
            err,
            FetchError::Api {
                code: 404,
                message: "not found".into()
            }
        );
    }
}
