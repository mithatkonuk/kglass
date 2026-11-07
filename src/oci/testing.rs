//! Test helper: a fake container CLI built from fixtures.

use std::sync::Mutex;

use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream::{self, BoxStream};
use serde_json::Value;

use super::engine::Runner;
use crate::cluster::fetch::FetchError;
use crate::text::SafeText;

/// A fake CLI: answers by the first argument and records every call.
#[derive(Default)]
pub(crate) struct Fake {
    pub(crate) calls: Mutex<Vec<Vec<String>>>,
    pub(crate) fail: Option<FetchError>,
    pub(crate) no_containers: bool,
    pub(crate) server_version: &'static str,
}

impl Runner for Fake {
    fn run(&self, args: Vec<String>) -> BoxFuture<'static, Result<Vec<u8>, FetchError>> {
        self.calls.lock().unwrap().push(args.clone());
        let reply = match (&self.fail, args[0].as_str()) {
            (Some(e), _) => Err(e.clone()),
            (None, "ps") if self.no_containers => Ok(Vec::new()),
            (None, "ps") => Ok(b"id-web\nid-worker\nid-migrate\nid-flaky\nid-fresh\nid-evil\n".to_vec()),
            (None, "container") => {
                let all: Vec<Value> = serde_json::from_str(include_str!("../../tests/fixtures/oci-inspect.json")).unwrap();
                let wanted: Vec<Value> = if args.len() == 4 && !args[3].starts_with("id-") {
                    all.into_iter().filter(|c| c["Name"] == format!("/{}", args[3])).collect()
                } else {
                    all
                };
                Ok(serde_json::to_vec(&wanted).unwrap())
            }
            (None, "volume") => Ok(br#"[{"Name":"shop-data","Driver":"local","Mountpoint":"/var/lib/docker/volumes/shop-data/_data"}]"#.to_vec()),
            (None, "version") if args[2].contains("Server") => Ok(format!("{}\n", self.server_version).into_bytes()),
            (None, "version") => Ok(b"5.8.0\n".to_vec()),
            _ => Err(FetchError::Unreachable("unexpected command".into())),
        };
        Box::pin(async move { reply })
    }

    fn stream(&self, args: Vec<String>) -> BoxStream<'static, Result<SafeText, FetchError>> {
        self.calls.lock().unwrap().push(args);
        stream::iter(vec![
            Ok(SafeText::from("line one")),
            Ok(SafeText::from("line two")),
        ])
        .boxed()
    }
}
