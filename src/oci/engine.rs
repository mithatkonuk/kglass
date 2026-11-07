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

/// Picker entry meaning "every compose project" (and containers without one).
pub const ALL_PROJECTS: &str = "(all projects)";
/// Container ids per `container inspect` call, to stay far below ARG_MAX.
const INSPECT_BATCH: usize = 200;

/// Split container ids into `container inspect` batches.
pub(crate) fn inspect_batches(ids: &[String]) -> Vec<Vec<String>> {
    ids.chunks(INSPECT_BATCH).map(<[String]>::to_vec).collect()
}

/// Remove host paths, the creating command line and volume options from `inspect`
/// output before display. Environment variables stay (shown as-is, like kubectl).
fn redact(value: &mut Value) {
    if let Some(config) = value.get_mut("Config").and_then(Value::as_object_mut) {
        config.remove("CreateCommand");
    }
    if let Some(host) = value.get_mut("HostConfig").and_then(Value::as_object_mut) {
        host.remove("Binds");
    }
    for mount in value
        .get_mut("Mounts")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        if mount.get("Type").and_then(Value::as_str) == Some("bind")
            && let Some(m) = mount.as_object_mut()
        {
            m.remove("Source");
        }
    }
    if let Some(options) = value.get_mut("Options").and_then(Value::as_object_mut) {
        for v in options.values_mut() {
            *v = Value::String("***".to_string());
        }
    }
}

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
        // The stream owns the child until the end: dropping the stream early kills
        // the process. When output ends, a failed exit (e.g. no such container) is
        // reported as an error instead of a silent end.
        let exit = stream::once(async move {
            match child.wait().await {
                Ok(status) if !status.success() => Some(Err(FetchError::Unreachable(
                    SafeText::from(format!("logs command ended with {status}")),
                ))),
                _ => None,
            }
        })
        .filter_map(futures::future::ready);
        let mut last_stderr: Option<SafeText> = None;
        merged
            .chain(exit)
            .map(move |item| match item {
                Err(FetchError::Unreachable(_)) if last_stderr.is_some() => Err(
                    FetchError::Unreachable(last_stderr.take().unwrap_or_default()),
                ),
                Ok(line) => {
                    last_stderr = Some(line.clone());
                    Ok(line)
                }
                other => other,
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

    /// `container inspect` for every container (running or not). Retried once when
    /// a container vanishes between `ps` and `inspect`.
    async fn inspect_all(&self) -> Result<Vec<Value>, FetchError> {
        match self.inspect_once().await {
            Ok(all) => Ok(all),
            Err(_) => self.inspect_once().await,
        }
    }

    async fn inspect_once(&self) -> Result<Vec<Value>, FetchError> {
        let ids = self.runner.run(args(&["ps", "-aq", "--no-trunc"])).await?;
        let ids: Vec<String> = String::from_utf8_lossy(&ids)
            .lines()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .collect();
        let mut all = Vec::new();
        for batch in inspect_batches(&ids) {
            let mut inspect = args(&["container", "inspect", "--"]);
            inspect.extend(batch);
            let json = self.runner.run(inspect).await?;
            let mut items: Vec<Value> = serde_json::from_slice(&json).map_err(|e| {
                FetchError::Unreachable(SafeText::from(format!("unexpected inspect output: {e}")))
            })?;
            all.append(&mut items);
        }
        Ok(all)
    }

    /// Names of all volumes; an error just means unused volumes are not listed.
    async fn volume_names(&self) -> Vec<String> {
        self.runner
            .run(args(&["volume", "ls", "--format", "{{.Name}}"]))
            .await
            .map(|out| {
                String::from_utf8_lossy(&out)
                    .lines()
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
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
        let unused_candidates = match scope {
            Scope::All => self.volume_names().await,
            Scope::Namespace(_) => Vec::new(),
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
            storage: Ok(convert::volumes_with_unused(&raw, &unused_candidates)),
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

    /// Picker entries: all projects, `-` (no project) when used, then each project.
    pub async fn projects(&self) -> Result<Vec<SafeText>, FetchError> {
        let all = self.inspect_all().await?;
        let mut projects: Vec<String> = all.iter().map(pod_project).collect();
        let has_unprojected = projects.iter().any(|p| p == NO_PROJECT);
        projects.retain(|p| p != NO_PROJECT);
        projects.sort();
        projects.dedup();
        let mut entries = vec![SafeText::from(ALL_PROJECTS)];
        if has_unprojected {
            entries.push(SafeText::from(NO_PROJECT));
        }
        entries.extend(projects.into_iter().map(SafeText::from));
        Ok(entries)
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
        let mut one = value
            .as_array()
            .and_then(|a| a.first())
            .cloned()
            .unwrap_or(value);
        redact(&mut one);
        serde_norway::to_string(&one)
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
        assert_eq!(lists.pods.unwrap().len(), 8);
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
        assert_eq!(names, ["shop-web-1", "shop-worker-1", "sick"]);
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
        let calls = fake.calls.lock().unwrap();
        assert!(
            calls.iter().all(|c| c[0] != "container"),
            "no inspect without containers: {calls:?}"
        );
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
        assert_eq!(projects, [ALL_PROJECTS, "-", "lab", "shop"]);
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
    async fn yaml_redacts_host_paths_create_command_and_volume_options() {
        let (e, _) = healthy();
        let y = e
            .inspect_yaml(&ResourceRef::Pod("shop-web-1".into()))
            .await
            .unwrap();
        assert!(
            !y.contains("/Users/someone"),
            "bind-mount host paths hidden\n{y}"
        );
        assert!(
            !y.contains("hunter2") && !y.contains("CreateCommand"),
            "create command hidden\n{y}"
        );
        assert!(
            y.contains("APP_MODE=demo"),
            "env shown as-is, like kubectl\n{y}"
        );
        assert!(
            y.contains("/var/lib/docker/volumes/shop-data/_data"),
            "named volume paths kept"
        );
        let v = e
            .inspect_yaml(&ResourceRef::Pvc("shop-data".into()))
            .await
            .unwrap();
        assert!(
            !v.contains("s3cr3t") && v.contains("password: '***'"),
            "{v}"
        );
    }

    #[tokio::test]
    async fn vanished_container_is_retried_once() {
        let (e, fake) = engine(Fake {
            fail_first_inspect: true.into(),
            server_version: "1",
            ..Fake::default()
        });
        let lists = e.fetch_lists(&Scope::All, now()).await;
        assert!(lists.pods.is_ok(), "{:?}", lists.pods);
        let inspects = fake
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c[0] == "container")
            .count();
        assert_eq!(inspects, 2);
    }

    #[test]
    fn inspect_is_batched() {
        let ids: Vec<String> = (0..450).map(|i| format!("id{i}")).collect();
        let batches = inspect_batches(&ids);
        assert_eq!(
            batches.iter().map(Vec::len).collect::<Vec<_>>(),
            [200, 200, 50]
        );
    }

    #[tokio::test]
    async fn projects_offer_all_and_no_project() {
        let (e, _) = healthy();
        let projects: Vec<String> = e
            .projects()
            .await
            .unwrap()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(projects, [ALL_PROJECTS, "-", "lab", "shop"]);
    }

    #[tokio::test]
    async fn failed_log_command_reports_an_error() {
        let runner = ProcessRunner::new("sh");
        let items: Vec<_> = runner
            .stream(vec![
                "-c".into(),
                "echo 'Error: No such container: x' >&2; exit 1".into(),
            ])
            .collect()
            .await;
        assert!(
            matches!(items.last(), Some(Err(FetchError::Unreachable(m))) if m.as_str().contains("No such container")),
            "{items:?}"
        );
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
