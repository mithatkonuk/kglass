//! Test helpers: a fake Kubernetes API server built from fixtures.

use std::time::Duration;

use http::{Request, Response};
use kube::Client;
use kube::client::Body;

pub(crate) const NS: &str = "shop";

pub(crate) fn list_json(kind: &str, items: &str) -> Vec<u8> {
    format!(r#"{{"apiVersion":"v1","kind":"{kind}","metadata":{{}},"items":{items}}}"#).into_bytes()
}

pub(crate) fn status_json(code: u16, reason: &str, message: &str) -> Vec<u8> {
    format!(
        r#"{{"kind":"Status","apiVersion":"v1","metadata":{{}},"status":"Failure","message":"{message}","reason":"{reason}","code":{code}}}"#
    )
    .into_bytes()
}

/// HTTP status and body returned by the fake API server.
pub(crate) type Reply = Option<(u16, Vec<u8>)>;

/// A fake API server: `route(path)` returns (status, body) or `None` to hang forever.
pub(crate) fn mock(route: fn(&str) -> Reply) -> Client {
    let service = tower::service_fn(move |req: Request<Body>| async move {
        match route(req.uri().path()) {
            Some((code, body)) => Ok::<_, std::convert::Infallible>(
                Response::builder()
                    .status(code)
                    .body(Body::from(body))
                    .unwrap(),
            ),
            None => {
                tokio::time::sleep(Duration::from_secs(3600)).await;
                unreachable!("request should time out first")
            }
        }
    });
    Client::new(service, NS)
}

pub(crate) fn healthy(path: &str) -> Reply {
    let fixture = |f: &str| match f {
        "pods" => include_str!("../../tests/fixtures/pods.json"),
        "services" => include_str!("../../tests/fixtures/services.json"),
        "endpointslices" => include_str!("../../tests/fixtures/endpointslices.json"),
        "pvcs" => include_str!("../../tests/fixtures/pvcs.json"),
        "pvs" => include_str!("../../tests/fixtures/pvs.json"),
        _ => include_str!("../../tests/fixtures/events.json"),
    };
    let body = match path {
        "/api/v1/namespaces/shop/pods" => list_json("PodList", fixture("pods")),
        "/api/v1/namespaces/shop/services" => list_json("ServiceList", fixture("services")),
        "/apis/discovery.k8s.io/v1/namespaces/shop/endpointslices" => {
            list_json("EndpointSliceList", fixture("endpointslices"))
        }
        "/api/v1/namespaces/shop/persistentvolumeclaims" => {
            list_json("PersistentVolumeClaimList", fixture("pvcs"))
        }
        "/api/v1/persistentvolumes" => list_json("PersistentVolumeList", fixture("pvs")),
        "/api/v1/namespaces/shop/events" => list_json("EventList", fixture("events")),
        "/api/v1/namespaces/shop/pods/api-1/log" => {
            b"line one\nbad \xff byte\nescape \x1b[2Jdone\n".to_vec()
        }
        "/api/v1/namespaces/shop/pods/api-1" => {
            br#"{"apiVersion":"v1","kind":"Pod","metadata":{"name":"api-1","managedFields":[{}]}}"#.to_vec()
        }
        "/version" => br#"{"major":"1","minor":"36","gitVersion":"v1.36.4+rke2r1","gitCommit":"","gitTreeState":"","buildDate":"","goVersion":"","compiler":"","platform":""}"#.to_vec(),
        "/api/v1/namespaces" => list_json(
            "NamespaceList",
            r#"[{"metadata":{"name":"default"}},{"metadata":{"name":"shop"}}]"#,
        ),
        _ => return Some((404, status_json(404, "NotFound", "not found"))),
    };
    Some((200, body))
}
