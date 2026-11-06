//! Kubeconfig resolution with kubectl's rules (design D15).
//!
//! Kubeconfig: `--kubeconfig` > `$KUBECONFIG` (list, merged, first wins) > `~/.kube/config`.
//! Context: `--context` > `current-context`. Namespace: `-n` > context namespace > `default`.
//! Kubeconfig files hold credentials, so nothing here derives `Debug` over them.

use std::fmt;
use std::path::{Path, PathBuf};

pub use kube::config::Kubeconfig;
use kube::config::{KubeConfigOptions, KubeconfigError};
use thiserror::Error;

use crate::text::SafeText;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("cannot read kubeconfig {path}: {reason}")]
    Read { path: String, reason: String },
    #[error("kubeconfig has no current-context; pass --context")]
    NoCurrentContext,
    #[error("context \"{0}\" not found in kubeconfig")]
    UnknownContext(String),
    #[error("cannot use context \"{context}\": {reason}")]
    Client { context: String, reason: String },
}

/// Describe a kubeconfig error without echoing file contents (which may hold tokens).
fn describe(err: &KubeconfigError) -> String {
    match err {
        KubeconfigError::ReadConfig(io, _) => io.to_string(),
        KubeconfigError::Parse(_) => "not valid kubeconfig YAML".to_string(),
        other => other.to_string(),
    }
}

fn read_file(path: &Path) -> Result<Kubeconfig, ConfigError> {
    Kubeconfig::read_from(path).map_err(|e| ConfigError::Read {
        path: path.display().to_string(),
        reason: describe(&e),
    })
}

/// Load the kubeconfig kubectl would use. `env` is the value of `$KUBECONFIG` and
/// `home` the user's home directory; both are parameters so tests can control them.
pub fn load_kubeconfig(
    explicit: Option<&Path>,
    env: Option<&str>,
    home: Option<&Path>,
) -> Result<Kubeconfig, ConfigError> {
    if let Some(path) = explicit {
        return read_file(path);
    }
    if let Some(list) = env {
        let paths: Vec<PathBuf> = std::env::split_paths(list)
            .filter(|p| !p.as_os_str().is_empty())
            .collect();
        if !paths.is_empty() {
            let mut merged = Kubeconfig::default();
            for path in &paths {
                merged = merged
                    .merge(read_file(path)?)
                    .map_err(|e| ConfigError::Read {
                        path: path.display().to_string(),
                        reason: describe(&e),
                    })?;
            }
            return Ok(merged);
        }
    }
    let default = home
        .map(|h| h.join(".kube").join("config"))
        .unwrap_or_else(|| PathBuf::from("~/.kube/config"));
    read_file(&default)
}

/// The cluster, context and namespace to show. `Debug` prints names only.
#[derive(Clone)]
pub struct Target {
    pub kubeconfig: Kubeconfig,
    pub context: String,
    pub namespace: String,
}

impl fmt::Debug for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Target")
            .field("context", &self.context)
            .field("namespace", &self.namespace)
            .finish_non_exhaustive()
    }
}

/// Pick the context and namespace according to D15.
pub fn resolve(
    kubeconfig: Kubeconfig,
    context: Option<&str>,
    namespace: Option<&str>,
) -> Result<Target, ConfigError> {
    let name = match context {
        Some(c) => c.to_string(),
        None => kubeconfig
            .current_context
            .clone()
            .ok_or(ConfigError::NoCurrentContext)?,
    };
    let ctx = kubeconfig
        .contexts
        .iter()
        .find(|c| c.name == name)
        .ok_or_else(|| ConfigError::UnknownContext(name.clone()))?;
    let namespace = namespace
        .map(str::to_string)
        .or_else(|| ctx.context.as_ref().and_then(|c| c.namespace.clone()))
        .unwrap_or_else(|| "default".to_string());
    Ok(Target {
        kubeconfig,
        context: name,
        namespace,
    })
}

/// A context as shown in the picker. Holds no credentials.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextInfo {
    pub name: SafeText,
    pub cluster: SafeText,
    pub server_host: SafeText,
    pub namespace: Option<SafeText>,
    pub hint: &'static str,
}

/// `https://host:port/path` → `host:port`.
fn host_of(server: &str) -> &str {
    let rest = server.split_once("://").map_or(server, |(_, r)| r);
    rest.split('/').next().unwrap_or(rest)
}

/// Best-effort platform hint from the server URL, shown next to each context.
fn hint_for(server: &str) -> &'static str {
    let host = host_of(server);
    if server.contains("/k8s/clusters/") {
        "rancher"
    } else if host.contains(".eks.amazonaws.com") {
        "eks"
    } else if host.contains(".azmk8s.io") {
        "aks"
    } else if host.starts_with("127.0.0.1")
        || host.starts_with("localhost")
        || host.starts_with("[::1]")
    {
        "local"
    } else {
        ""
    }
}

/// All contexts in kubeconfig order.
pub fn contexts(kubeconfig: &Kubeconfig) -> Vec<ContextInfo> {
    kubeconfig
        .contexts
        .iter()
        .map(|named| {
            let ctx = named.context.as_ref();
            let cluster = ctx.map_or("", |c| c.cluster.as_str());
            let server = kubeconfig
                .clusters
                .iter()
                .find(|c| c.name == cluster)
                .and_then(|c| c.cluster.as_ref())
                .and_then(|c| c.server.as_deref())
                .unwrap_or("");
            ContextInfo {
                name: SafeText::from(named.name.as_str()),
                cluster: SafeText::from(cluster),
                server_host: SafeText::from(host_of(server)),
                namespace: ctx.and_then(|c| c.namespace.as_deref()).map(SafeText::from),
                hint: hint_for(server),
            }
        })
        .collect()
}

/// Build an API client for `target`. Does not contact the cluster.
pub async fn client_for(target: &Target) -> Result<kube::Client, ConfigError> {
    let options = KubeConfigOptions {
        context: Some(target.context.clone()),
        ..KubeConfigOptions::default()
    };
    let client_error = |reason: String| ConfigError::Client {
        context: target.context.clone(),
        reason,
    };
    let mut config = kube::Config::from_custom_kubeconfig(target.kubeconfig.clone(), &options)
        .await
        .map_err(|e| client_error(describe(&e)))?;
    config.default_namespace = target.namespace.clone();
    kube::Client::try_from(config).map_err(|e| client_error(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    fn env_list() -> String {
        std::env::join_paths([fixture("kubeconfig-a.yaml"), fixture("kubeconfig-b.yaml")])
            .unwrap()
            .into_string()
            .unwrap()
    }

    fn merged() -> Kubeconfig {
        load_kubeconfig(None, Some(&env_list()), None).unwrap()
    }

    #[test]
    fn merges_kubeconfig_list_first_wins() {
        let kc = merged();
        assert_eq!(kc.current_context.as_deref(), Some("rancher-01"));
        let names: Vec<_> = contexts(&kc)
            .into_iter()
            .map(|c| c.name.to_string())
            .collect();
        assert_eq!(names, ["rancher-01", "kind-local", "prod-eks"]);
    }

    #[test]
    fn explicit_path_beats_env() {
        let kc =
            load_kubeconfig(Some(&fixture("kubeconfig-b.yaml")), Some(&env_list()), None).unwrap();
        assert_eq!(kc.current_context.as_deref(), Some("kind-local"));
    }

    #[test]
    fn home_default_used_without_env() {
        let home = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/home");
        let err = load_kubeconfig(None, None, Some(&home)).unwrap_err();
        assert!(err.to_string().contains(".kube/config"), "{err}");
    }

    #[test]
    fn context_namespace_used_when_no_flag() {
        let t = resolve(merged(), None, None).unwrap();
        assert_eq!(
            (t.context.as_str(), t.namespace.as_str()),
            ("rancher-01", "team-a")
        );
    }

    #[test]
    fn flag_overrides_context_and_namespace() {
        let t = resolve(merged(), Some("prod-eks"), None).unwrap();
        assert_eq!(
            (t.context.as_str(), t.namespace.as_str()),
            ("prod-eks", "shop")
        );
        let t = resolve(merged(), Some("prod-eks"), Some("ops")).unwrap();
        assert_eq!(t.namespace, "ops");
    }

    #[test]
    fn falls_back_to_default_namespace() {
        let t = resolve(merged(), Some("kind-local"), None).unwrap();
        assert_eq!(t.namespace, "default");
    }

    #[test]
    fn unknown_context_is_error() {
        let err = resolve(merged(), Some("nope"), None).unwrap_err();
        assert!(err.to_string().contains("nope"), "{err}");
    }

    #[test]
    fn missing_current_context_is_error() {
        let mut kc = merged();
        kc.current_context = None;
        assert!(matches!(
            resolve(kc, None, None),
            Err(ConfigError::NoCurrentContext)
        ));
    }

    #[test]
    fn hints_from_server_url() {
        let kc = merged();
        let hints: Vec<_> = contexts(&kc)
            .iter()
            .map(|c| (c.name.to_string(), c.hint, c.server_host.to_string()))
            .collect();
        assert_eq!(
            hints,
            [
                (
                    "rancher-01".into(),
                    "rancher",
                    "rancher.example.test".into()
                ),
                ("kind-local".into(), "local", "127.0.0.1:6443".into()),
                (
                    "prod-eks".into(),
                    "eks",
                    "ABCDEF.gr7.us-east-1.eks.amazonaws.com".into()
                ),
            ]
        );
    }

    #[tokio::test]
    async fn client_builds_without_contacting_cluster() {
        let t = resolve(merged(), Some("kind-local"), None).unwrap();
        let client = client_for(&t).await.unwrap();
        assert_eq!(client.default_namespace(), "default");
    }

    #[test]
    fn debug_output_has_no_token() {
        let kc = merged();
        let infos = format!("{:?}", contexts(&kc));
        let target = format!("{:?}", resolve(kc, None, None).unwrap());
        for s in [infos, target] {
            assert!(!s.contains("test-token-not-real"), "{s}");
        }
    }
}
