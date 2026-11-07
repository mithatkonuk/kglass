//! Runs the container CLI (no shell) and turns its output into kglass data.
//!
//! Only read-only subcommands are ever run: `ps`, `container inspect`,
//! `volume inspect`, `logs` and `version`. Names are always passed after `--`,
//! so a name can never be read as an option.

use std::process::Stdio;
use std::sync::Arc;

use futures::future::BoxFuture;
use futures::stream::{self, BoxStream};
use futures::{FutureExt, StreamExt};
use serde_json::Value;
use tokio::process::Command;
use tokio::time::timeout;
use tokio_util::compat::TokioAsyncReadCompatExt;

use super::convert::{self, NO_PROJECT, Timestamp};
use crate::cluster::fetch::{FetchError, Lists, REQUEST_TIMEOUT, Scope, lines_lossy};
use crate::model::ResourceRef;
use crate::text::SafeText;

/// Runs one CLI invocation. A trait so tests can use a fake engine.
pub trait Runner: Send + Sync {
    /// Run to completion and return stdout; a non-zero exit is an error.
    fn run(&self, args: Vec<String>) -> BoxFuture<'static, Result<Vec<u8>, FetchError>>;
    /// Stream stdout and stderr lines until the process exits or the stream is dropped.
    fn stream(&self, args: Vec<String>) -> BoxStream<'static, Result<SafeText, FetchError>>;
}

/// Runs the real CLI binary (`docker`, `podman`, `nerdctl`, …) without a shell.
pub struct ProcessRunner {
    command: String,
}

impl ProcessRunner {
    pub fn new(command: impl Into<String>) -> Self {
        ProcessRunner {
            command: command.into(),
        }
    }

    fn not_found(&self) -> FetchError {
        FetchError::Unreachable(SafeText::from(format!(
            "`{}` command not found",
            self.command
        )))
    }
}

impl Runner for ProcessRunner {
    fn run(&self, args: Vec<String>) -> BoxFuture<'static, Result<Vec<u8>, FetchError>> {
        let mut cmd = Command::new(&self.command);
        cmd.args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let not_found = self.not_found();
        async move {
            let output = match timeout(REQUEST_TIMEOUT, cmd.output()).await {
                Err(_) => return Err(FetchError::Timeout),
                Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => return Err(not_found),
                Ok(Err(e)) => return Err(FetchError::Unreachable(SafeText::from(e.to_string()))),
                Ok(Ok(output)) => output,
            };
            if output.status.success() {
                return Ok(output.stdout);
            }
            // The first stderr line explains it, e.g. "Cannot connect to the Docker daemon".
            let stderr = String::from_utf8_lossy(&output.stderr);
            let reason = stderr
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .map_or_else(|| output.status.to_string(), str::to_string);
            Err(FetchError::Unreachable(SafeText::from(reason)))
        }
        .boxed()
    }

    fn stream(&self, args: Vec<String>) -> BoxStream<'static, Result<SafeText, FetchError>> {
        let spawned = Command::new(&self.command)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return stream::once(futures::future::ready(Err(self.not_found()))).boxed();
            }
            Err(e) => {
                let err = FetchError::Unreachable(SafeText::from(e.to_string()));
                return stream::once(futures::future::ready(Err(err))).boxed();
            }
        };
        let stdout = child
            .stdout
            .take()
            .map(|o| lines_lossy(futures::io::BufReader::new(o.compat())).boxed());
        let stderr = child
            .stderr
            .take()
            .map(|e| lines_lossy(futures::io::BufReader::new(e.compat())).boxed());
        let merged = stream::select(
            stdout.unwrap_or_else(|| stream::empty().boxed()),
            stderr.unwrap_or_else(|| stream::empty().boxed()),
        );
        // The stream owns the child: dropping the stream kills the process.
        merged
            .map(move |line| {
                let _owner = &child;
                line
            })
            .boxed()
    }
}

/// A local container engine reached through its CLI.
#[derive(Clone)]
pub struct Engine {
    /// The CLI name, e.g. `docker` or `podman`.
    pub command: String,
    runner: Arc<dyn Runner>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("command", &self.command)
            .finish_non_exhaustive()
    }
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_string()).collect()
}

impl Engine {
    pub fn new(command: impl Into<String>) -> Self {
        let command = command.into();
        Engine {
            runner: Arc::new(ProcessRunner::new(command.clone())),
            command,
        }
    }

    pub fn with_runner(command: impl Into<String>, runner: Arc<dyn Runner>) -> Self {
        Engine {
            command: command.into(),
            runner,
        }
    }

    /// `container inspect` for every container (running or not).
    async fn inspect_all(&self) -> Result<Vec<Value>, FetchError> {
        let ids = self.runner.run(args(&["ps", "-aq", "--no-trunc"])).await?;
        let ids: Vec<String> = String::from_utf8_lossy(&ids)
            .lines()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut inspect = args(&["container", "inspect", "--"]);
        inspect.extend(ids);
        let json = self.runner.run(inspect).await?;
        serde_json::from_slice(&json).map_err(|e| {
            FetchError::Unreachable(SafeText::from(format!("unexpected inspect output: {e}")))
        })
    }

    /// Containers, published ports and volumes. `Scope::Namespace` filters by
    /// compose project. Local engines have no events.
    pub async fn fetch_lists(&self, scope: &Scope, now: Timestamp) -> Lists {
        let raw = match self.inspect_all().await {
            Ok(raw) => raw,
            Err(e) => {
                return Lists {
                    pods: Err(e.clone()),
                    services: Err(e.clone()),
                    storage: Err(e.clone()),
                    storage_partial: None,
                    events: Err(e),
                    truncated: Vec::new(),
                };
            }
        };
        let raw: Vec<Value> = match scope {
            Scope::All => raw,
            Scope::Namespace(project) => raw
                .into_iter()
                .filter(|c| pod_project(c) == project.as_str())
                .collect(),
        };
        Lists {
            pods: Ok(raw.iter().filter_map(|c| convert::pod(c, now)).collect()),
            services: Ok(raw.iter().filter_map(convert::published).collect()),
            storage: Ok(convert::volumes(&raw)),
            storage_partial: None,
            events: Ok(Vec::new()),
            truncated: Vec::new(),
        }
    }

    /// `docker 29.9.2`: the engine's server version, or the CLI's own version.
    pub async fn version(&self) -> Result<SafeText, FetchError> {
        for template in ["{{.Server.Version}}", "{{.Client.Version}}"] {
            let out = self
                .runner
                .run(args(&["version", "--format", template]))
                .await?;
            let version = String::from_utf8_lossy(&out).trim().to_string();
            if !version.is_empty() && version != "<no value>" {
                return Ok(SafeText::from(format!("{} {version}", self.command)));
            }
        }
        Ok(SafeText::from(self.command.as_str()))
    }

    /// Compose projects in use, for the namespace picker.
    pub async fn projects(&self) -> Result<Vec<SafeText>, FetchError> {
        let mut projects: Vec<String> = self
            .inspect_all()
            .await?
            .iter()
            .map(|c| pod_project(c).to_string())
            .filter(|p| p != NO_PROJECT)
            .collect();
        projects.sort();
        projects.dedup();
        Ok(projects.into_iter().map(SafeText::from).collect())
    }

    /// `inspect` output of a container (pods and published ports) or volume, as YAML.
    pub async fn inspect_yaml(&self, target: &ResourceRef) -> Result<String, FetchError> {
        let name = target.name().to_string();
        let mut command = match target {
            ResourceRef::Pod(_) | ResourceRef::Service(_) => args(&["container", "inspect", "--"]),
            ResourceRef::Pvc(_) => args(&["volume", "inspect", "--"]),
            ResourceRef::Image(_) | ResourceRef::Pv(_) => return Ok(String::new()),
        };
        command.push(name);
        let json = self.runner.run(command).await?;
        let value: Value = serde_json::from_slice(&json).map_err(|e| {
            FetchError::Unreachable(SafeText::from(format!("unexpected inspect output: {e}")))
        })?;
        let one = value.as_array().and_then(|a| a.first()).unwrap_or(&value);
        serde_norway::to_string(one)
            .map_err(|e| FetchError::Unreachable(SafeText::from(e.to_string())))
    }

    /// Follow a container's logs from the last `tail` lines (stdout and stderr).
    pub fn log_lines(
        &self,
        container: &str,
        tail: i64,
    ) -> BoxStream<'static, Result<SafeText, FetchError>> {
        let mut command = args(&["logs", "--follow", "--tail"]);
        command.push(tail.to_string());
        command.push("--".to_string());
        command.push(container.to_string());
        self.runner.stream(command)
    }
}

fn pod_project(c: &Value) -> String {
    convert::project_of(c).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oci::testing::Fake;
    use futures::StreamExt;

    fn engine(fake: Fake) -> (Engine, Arc<Fake>) {
        let fake = Arc::new(fake);
        (Engine::with_runner("docker", fake.clone()), fake)
    }

    fn healthy() -> (Engine, Arc<Fake>) {
        engine(Fake {
            server_version: "29.9.2",
            ..Fake::default()
        })
    }

    fn now() -> Timestamp {
        "2026-11-07T12:00:00Z".parse().unwrap()
    }

    #[tokio::test]
    async fn lists_containers_ports_and_volumes() {
        let (e, fake) = healthy();
        let lists = e.fetch_lists(&Scope::All, now()).await;
        assert_eq!(lists.pods.unwrap().len(), 6);
        assert_eq!(lists.services.unwrap().len(), 1);
        assert_eq!(lists.storage.unwrap()[0].name.as_str(), "shop-data");
        assert_eq!(lists.events, Ok(Vec::new()));
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls[0], ["ps", "-aq", "--no-trunc"]);
        assert_eq!(
            &calls[1][..3],
            ["container", "inspect", "--"],
            "ids after --"
        );
    }

    #[tokio::test]
    async fn namespace_filters_by_compose_project() {
        let (e, _) = healthy();
        let lists = e.fetch_lists(&Scope::Namespace("shop".into()), now()).await;
        let names: Vec<String> = lists
            .pods
            .unwrap()
            .iter()
            .map(|p| p.name.to_string())
            .collect();
        assert_eq!(names, ["shop-web-1", "shop-worker-1"]);
        assert_eq!(lists.storage.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn no_containers_skips_inspect() {
        let (e, fake) = engine(Fake {
            no_containers: true,
            ..Fake::default()
        });
        let lists = e.fetch_lists(&Scope::All, now()).await;
        assert_eq!(lists.pods, Ok(Vec::new()));
        assert_eq!(fake.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn engine_errors_fail_every_pane() {
        let down = FetchError::Unreachable("Cannot connect to the Docker daemon".into());
        let (e, _) = engine(Fake {
            fail: Some(down.clone()),
            ..Fake::default()
        });
        let lists = e.fetch_lists(&Scope::All, now()).await;
        assert_eq!(lists.pods, Err(down.clone()));
        assert_eq!(lists.storage, Err(down.clone()));
        assert_eq!(e.version().await, Err(down));
    }

    #[tokio::test]
    async fn version_names_the_command_and_falls_back_to_client() {
        let (e, _) = healthy();
        assert_eq!(e.version().await.unwrap().as_str(), "docker 29.9.2");
        let (podman, _) = engine(Fake {
            server_version: "",
            ..Fake::default()
        });
        assert_eq!(podman.version().await.unwrap().as_str(), "docker 5.8.0");
    }

    #[tokio::test]
    async fn projects_are_unique_and_sorted() {
        let (e, _) = healthy();
        let projects: Vec<String> = e
            .projects()
            .await
            .unwrap()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(projects, ["lab", "shop"]);
    }

    #[tokio::test]
    async fn yaml_for_containers_and_volumes() {
        let (e, fake) = healthy();
        let y = e
            .inspect_yaml(&ResourceRef::Pod("shop-web-1".into()))
            .await
            .unwrap();
        assert!(y.contains("Image: nginx:1.27"), "{y}");
        let v = e
            .inspect_yaml(&ResourceRef::Pvc("shop-data".into()))
            .await
            .unwrap();
        assert!(v.contains("Driver: local"), "{v}");
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls[0], ["container", "inspect", "--", "shop-web-1"]);
        assert_eq!(calls[1], ["volume", "inspect", "--", "shop-data"]);
    }

    #[tokio::test]
    async fn logs_follow_with_tail_and_separator() {
        let (e, fake) = healthy();
        let lines: Vec<_> = e.log_lines("shop-web-1", 500).collect().await;
        assert_eq!(lines.len(), 2);
        assert_eq!(
            fake.calls.lock().unwrap()[0],
            ["logs", "--follow", "--tail", "500", "--", "shop-web-1"]
        );
    }

    #[tokio::test]
    async fn missing_command_is_reported() {
        let runner = ProcessRunner::new("kglass-no-such-command-zz");
        let err = runner.run(vec!["ps".into()]).await.unwrap_err();
        assert_eq!(
            err,
            FetchError::Unreachable("`kglass-no-such-command-zz` command not found".into())
        );
    }

    #[tokio::test]
    async fn failing_command_reports_its_stderr() {
        let runner = ProcessRunner::new("sh");
        let err = runner
            .run(vec![
                "-c".into(),
                "echo 'Cannot connect to the daemon' >&2; exit 1".into(),
            ])
            .await
            .unwrap_err();
        assert_eq!(
            err,
            FetchError::Unreachable("Cannot connect to the daemon".into())
        );
    }

    #[tokio::test]
    async fn real_process_stream_merges_and_sanitizes() {
        let runner = ProcessRunner::new("sh");
        let mut lines: Vec<String> = runner
            .stream(vec![
                "-c".into(),
                "printf 'out \\033[31mred\\n'; echo err >&2".into(),
            ])
            .map(|l| l.unwrap().to_string())
            .collect()
            .await;
        lines.sort();
        assert_eq!(lines, ["err", "out red"]);
    }
}
