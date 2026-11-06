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
use crate::model::{KubeEvent, Pod, ResourceRef, Service, StorageItem};
use crate::text::SafeText;

/// Upper bound for any single request (design D5).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// Items requested per page.
const PAGE_SIZE: u32 = 500;
/// Stop paging after this many items (large clusters; see design D13).
const MAX_ITEMS: usize = 5000;
/// Longest log line kept; the rest of the line is dropped.
pub const MAX_LOG_LINE: usize = 64 * 1024;

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

/// List all items (paged), stopping at [`MAX_ITEMS`].
async fn list<K>(api: Api<K>, action: Action<'_>) -> Result<Vec<K>, FetchError>
where
    K: Clone + DeserializeOwned + Debug,
{
    let mut items = Vec::new();
    let mut params = ListParams::default().limit(PAGE_SIZE);
    loop {
        let page = bounded(action, api.list(&params)).await?;
        items.extend(page.items);
        match page.metadata.continue_ {
            Some(token) if !token.is_empty() && items.len() < MAX_ITEMS => {
                params = params.continue_token(&token);
            }
            _ => return Ok(items),
        }
    }
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
}

pub async fn fetch_lists(client: &Client, ns: &str, now: Timestamp) -> Lists {
    let scope = format!("namespace \"{ns}\"");
    let in_ns = |resource| Action {
        verb: "list",
        resource,
        scope: &scope,
    };
    let cluster = |resource| Action {
        verb: "list",
        resource,
        scope: "cluster",
    };
    let (pods, services, slices, pvcs, pvs, events) = tokio::join!(
        list(
            Api::<k8s::Pod>::namespaced(client.clone(), ns),
            in_ns("pods")
        ),
        list(
            Api::<k8s::Service>::namespaced(client.clone(), ns),
            in_ns("services")
        ),
        list(
            Api::<EndpointSlice>::namespaced(client.clone(), ns),
            in_ns("endpointslices")
        ),
        list(
            Api::<k8s::PersistentVolumeClaim>::namespaced(client.clone(), ns),
            in_ns("persistentvolumeclaims")
        ),
        list(
            Api::<k8s::PersistentVolume>::all(client.clone()),
            cluster("persistentvolumes")
        ),
        list(
            Api::<k8s::Event>::namespaced(client.clone(), ns),
            in_ns("events")
        ),
    );
    let raw_pods = pods.as_deref().unwrap_or_default();
    let services = services.and_then(|svcs| {
        let slices = slices?;
        Ok(svcs.iter().map(|s| convert::service(s, &slices)).collect())
    });
    let (storage, storage_partial) = match (pvcs, pvs) {
        (Err(e), _) => (Err(e), None),
        (Ok(claims), volumes) => {
            let mut items: Vec<StorageItem> =
                claims.iter().map(|c| convert::pvc(c, raw_pods)).collect();
            let partial = match volumes {
                Ok(vs) => {
                    // PVs are cluster-scoped; show only those claimed from this namespace.
                    items.extend(
                        vs.iter()
                            .filter(|v| claim_namespace(v) == Some(ns))
                            .map(convert::pv),
                    );
                    None
                }
                Err(e) => Some(e),
            };
            (Ok(items), partial)
        }
    };
    Lists {
        pods: pods.map(|ps| ps.iter().map(|p| convert::pod(p, now)).collect()),
        services,
        storage,
        storage_partial,
        events: events.map(|es| es.iter().filter_map(|e| convert::event(e, now)).collect()),
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
    ns: &str,
    target: &ResourceRef,
) -> Result<String, FetchError> {
    let scope = format!("namespace \"{ns}\"");
    let get = |resource| Action {
        verb: "get",
        resource,
        scope: &scope,
    };
    let name = target.name().as_str();
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
    let all = list(Api::<k8s::Namespace>::all(client.clone()), action).await?;
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
) -> impl Stream<Item = Result<SafeText, FetchError>> + Send {
    let open = async move {
        let api = Api::<k8s::Pod>::namespaced(client, &ns);
        let params = LogParams {
            container: Some(container),
            follow: true,
            tail_lines: Some(tail),
            ..LogParams::default()
        };
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

/// Split a byte stream into sanitized lines of at most [`MAX_LOG_LINE`] bytes.
fn lines_lossy<R>(reader: R) -> impl Stream<Item = Result<SafeText, FetchError>> + Send
where
    R: AsyncBufRead + Unpin + Send,
{
    stream::try_unfold(reader, |mut reader| async move {
        let mut line = Vec::new();
        let mut saw_any = false;
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
        let text = SafeText::from(String::from_utf8_lossy(&line).as_ref());
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
        let lists = fetch_lists(&mock(healthy), NS, now()).await;
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
        let lists = fetch_lists(&mock(route), NS, now()).await;
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
        let lists = fetch_lists(&mock(route), NS, now()).await;
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
        let lists = fetch_lists(&mock(route), NS, now()).await;
        assert_eq!(lists.pods, Err(FetchError::Timeout));
        assert_eq!(lists.storage, Err(FetchError::Timeout));
    }

    #[tokio::test]
    async fn log_lines_sanitized_and_lossy() {
        let lines: Vec<_> = log_lines(mock(healthy), NS.into(), "api-1".into(), "api".into(), 500)
            .collect()
            .await;
        let lines: Vec<String> = lines.into_iter().map(|l| l.unwrap().to_string()).collect();
        assert_eq!(lines, ["line one", "bad \u{fffd} byte", "escape done"]);
    }

    #[tokio::test]
    async fn very_long_log_line_is_capped() {
        let reader =
            futures::io::Cursor::new([vec![b'x'; MAX_LOG_LINE * 2], b"\nnext\n".to_vec()].concat());
        let lines: Vec<_> = lines_lossy(reader).collect().await;
        let first = lines[0].as_ref().unwrap();
        assert_eq!(first.as_str().len(), MAX_LOG_LINE);
        assert_eq!(lines[1].as_ref().unwrap().as_str(), "next");
    }

    #[tokio::test]
    async fn yaml_version_and_namespaces() {
        let client = mock(healthy);
        let y = fetch_yaml(&client, NS, &ResourceRef::Pod("api-1".into()))
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
    async fn missing_object_maps_to_api_error() {
        let err = fetch_yaml(&mock(healthy), NS, &ResourceRef::Service("gone".into()))
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
