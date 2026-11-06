# Kubeglass Phase 2 — Live Cluster Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Plain `kglass` opens the cluster and namespace kubectl would use (D15) and shows **real** data: pod, image, service and storage lists, details, YAML, events and live pod logs. It refreshes every 10s, and every failure (403, timeout, disconnect) is shown on the pane where it happened. Sample data stays available only behind `--demo`.

**Architecture:** functional core, imperative shell.
- **Core (no I/O):** `App::update(AppEvent)` stays pure. Instead of doing I/O it queues `Command`s (`app.take_commands()`).
- **Shell:** the async runtime in `src/runtime.rs` executes those commands with kube-rs on tokio and feeds the results back as `AppEvent::Data(..)`.
- **Inputs:** terminal input arrives from a dedicated blocking thread over a tokio channel. Ticks and unix signals come from `tokio::select!`.
- **Kubernetes code** lives in `src/cluster/`:
  - `config.rs`: kubeconfig resolution and contexts
  - `convert.rs`: k8s-openapi objects → `model`, pure and fixture-tested
  - `fetch.rs`: async list, YAML and log calls with timeouts, mapping errors to `FetchError`
- **Tests** mock the API with `kube::Client::new(tower::service_fn(..))`, so they never need a cluster.

**Tech Stack:**
- `kube 4.2` (default features: client, rustls-tls, ring) and `k8s-openapi 0.28` (feature `latest`)
- `tokio 1` (rt-multi-thread, macros, sync, time, signal)
- `futures 0.3`, `serde_norway 0.9`, `serde_json 1`
- Dev-only: `tower 0.5` (util), `http 1`

Picking kube 4.2 over 5.0 was my call: 5.0 was released 5 days ago. Raising MSRV to **1.89** was also my call, because kube 4.2 needs it.

**Spec:** [`design.md`](design.md) §3–§5, D11–D15, and §4.3 edge states.

## Scope decisions (my calls, recorded in design.md)

- **Logs and YAML moved from phase 3 into phase 2.** Otherwise real pods would show fake logs. Phase 3 keeps the rest: log search, previous-container logs, ghost rows, Enter-to-follow relationships, and the full-screen startup auth screen.
- **Q7 → session only.** Switching context or namespace inside kglass never writes kubeconfig, like `kubectl --context/-n`. kglass stays read-only even for local files.
- **`-A/--all-namespaces` is deferred to phase 3.** It needs namespace-qualified `ResourceRef`s. For now it exits with a clear message.
- **The image list comes from pods only (Q4 default).**

## Global Constraints

- **Read-only.** Only `list`, `get`, `watch` and `pods/log` calls. Never fetch Secrets.
- **Kubeconfig resolution** (D15), in order:
  - Kubeconfig: `--kubeconfig` > `$KUBECONFIG` (colon list, merged) > `~/.kube/config`.
  - Context: `--context` > `current-context`.
  - Namespace: `-n` > the context's namespace > `default`.
- **Defaults:**
  - refresh `every 10s` (40 ticks of 250ms)
  - request timeout `15s`
  - log tail `500`, buffer cap `5000`
- **Credentials:** never shown, logged or put in `Debug` output. `ContextInfo` holds no credential fields.
- **Cluster text** goes through `SafeText` at the conversion boundary (`convert.rs`) and the log ingest boundary.
- **Each pane has its own state.** One pane failing never blanks another or the logs.
- **Stale responses** (from an older context, namespace or selection generation) are dropped.
- **Gate per task:** `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.

## Review Focus

1. **Expired or invalid Rancher token** (401 on every call): the header shows `● Disconnected` with the API message and a hint to regenerate the kubeconfig in Rancher. Tested in Task 4 (mapping) and Task 5 (state).
2. **403 on one resource only** (e.g. PVs at cluster scope): that pane shows `Forbidden (403)` with the verb, resource and scope, and other panes stay usable. Tested in Task 4 and Task 5.
3. **Selected pod deleted between refreshes:** the viewer shows `no longer present`, the log stream ends, and nothing panics. Tested in Task 5.
4. **Context or namespace switch while requests are in flight:** late responses for the old generation are ignored. Tested in Task 5.
5. **Log line with escape sequences or invalid UTF-8:** sanitized, with lossy decoding. Tested in Task 4.

---

### Task 1: Command queue in the core (no new deps)

**Files:** modify `src/app.rs`; add `src/command.rs`.

**Interfaces:**
- **Produces:**
  - `pub enum Command { Refresh { generation: u64 }, StartLogs { generation: u64, pod: SafeText, container: SafeText }, StopLogs, FetchYaml { generation: u64, target: ResourceRef }, FetchNamespaces, SwitchTarget { context: SafeText, namespace: SafeText } }`
  - `App::take_commands(&mut self) -> Vec<Command>`
  - `pub enum DataSource { Sample, Live }`
  - `App::new_live(target: TargetLabels) -> App` starts empty: every pane is `Loading`, and a `Refresh` is queued.
- **Behaviour:**
  - `select` of a pod in Live mode queues `StartLogs`, and selecting something else queues `StopLogs`.
  - On the YAML tab it queues `FetchYaml`.
  - In Sample mode, behaviour is exactly as in phase 1 (fake logs, `sample_yaml`).

**Tests** (in `app.rs`):
- `live_app_starts_loading_and_requests_refresh`
- `selecting_pod_in_live_mode_starts_logs`
- `selecting_service_stops_logs`
- `yaml_tab_requests_yaml_once`
- `sample_mode_queues_no_commands`

---

### Task 2: Dependencies, kubeconfig resolution, contexts

**Files:** `Cargo.toml` (deps, MSRV 1.89); create `src/cluster/mod.rs` and `src/cluster/config.rs`; add fixtures `tests/fixtures/kubeconfig-a.yaml` and `kubeconfig-b.yaml` (fake servers, token `test-token-not-real`).

**Interfaces:**
- `pub struct ContextInfo { pub name: SafeText, pub cluster: SafeText, pub server_host: SafeText, pub namespace: Option<SafeText>, pub hint: &'static str }`
  - `hint` is guessed from the server: `rancher` (URL path `/k8s/clusters/`), `eks`, `gke`, `aks`, `kind`/`local` for localhost or 127.0.0.1, otherwise `""`.
- `pub struct Target { pub kubeconfig: kube::config::Kubeconfig, pub context: String, pub namespace: String }`. Its custom `Debug` prints only the context and namespace.
- `pub fn load_kubeconfig(explicit: Option<&Path>, env: Option<&str>, home: Option<&Path>) -> Result<Kubeconfig, ConfigError>`: merges the colon list in order, so the first file wins.
- `pub fn resolve(kc: Kubeconfig, context: Option<&str>, namespace: Option<&str>) -> Result<Target, ConfigError>`
- `pub fn contexts(kc: &Kubeconfig) -> Vec<ContextInfo>`
- `pub async fn client_for(target: &Target) -> Result<kube::Client, ConfigError>`

**Tests:**
- `merges_kubeconfig_list_first_wins`: `KUBECONFIG=a:b`, where a sets `current-context`.
- `context_namespace_used_when_no_flag`
- `flag_overrides_context_and_namespace`
- `falls_back_to_default_namespace`
- `unknown_context_is_error`
- `rancher_hint_from_url`
- `debug_output_has_no_token`: format `Target` and `ContextInfo`, then assert `test-token-not-real` is absent.

---

### Task 3: Conversion from k8s-openapi to model

**Files:** create `src/cluster/convert.rs`; fixtures under `tests/fixtures/*.json` (pods covering running, crashloop, pending, succeeded, terminating and init containers; a service with EndpointSlices; a PVC and a PV; events).

**Interfaces:**
- `pub fn pod(p: &k8s::Pod, now: Timestamp) -> Pod`. Status follows kubectl:
  - a waiting reason `CrashLoopBackOff` → CrashLoopBackOff
  - phase Succeeded → Completed
  - phase Failed → Failed
  - phase Pending → Pending
  - otherwise Running
  - `waiting_reason` is set from the container state.
- `pub fn service(s: &k8s::Service, slices: &[EndpointSlice]) -> Service`: `ready_endpoints` counts endpoints whose `conditions.ready != false` in slices labelled `kubernetes.io/service-name=<name>`.
- `pub fn pvc(c: &k8s::PersistentVolumeClaim, pods: &[k8s::Pod]) -> StorageItem`: `mounted_by` comes from pod volumes whose `persistentVolumeClaim.claimName` matches. Access modes are abbreviated (RWO/ROX/RWX/RWOP).
- `pub fn pv(v: &k8s::PersistentVolume) -> StorageItem`: `bound_to` = `ns/name` from `claimRef`.
- `pub fn event(e: &k8s::Event, now) -> Option<KubeEvent>`: maps `involvedObject.kind` Pod/Service/PersistentVolumeClaim/PersistentVolume.
- `pub fn yaml_without_managed_fields<T: Serialize>(obj: &T) -> String`

**Tests:** each fixture maps to the exact expected model values. Also:
- `escape_sequences_in_names_are_sanitized`
- `terminating_pod_shows_terminating`: add `PodStatus::Terminating` (symbol `◌`, warn).
- `crashloop_from_waiting_reason`
- `ready_counts_regular_containers`

---

### Task 4: Async fetch layer with error mapping

**Files:** create `src/cluster/fetch.rs`.

**Interfaces:**
- `pub enum FetchError { Unauthorized(SafeText), Forbidden { verb: &'static str, resource: &'static str, scope: SafeText, message: SafeText }, Timeout, Unreachable(SafeText), Api { code: u16, message: SafeText } }` with `Display`.
- `pub struct Lists { pub pods, pub services, pub storage, pub events }`. Each is a `Result<Vec<..>, FetchError>`, fetched concurrently with `tokio::join!` and a 15s timeout each.
  - PVs (cluster scope) and PVCs are fetched separately and merged. If PVs are forbidden, the PVCs are still returned, with a partial-error note.
- `pub async fn fetch_lists(client: &Client, ns: &str) -> Lists`
- `pub async fn fetch_yaml(client, ns, target: &ResourceRef) -> Result<String, FetchError>`
- `pub fn log_stream(client, ns, pod, container, tail: i64) -> impl Stream<Item = Result<SafeText, FetchError>>`: lines are decoded lossily as UTF-8 and pass through `SafeText`.
- `pub async fn server_version(client) -> Result<SafeText, FetchError>`
- `pub async fn namespaces(client) -> Result<Vec<SafeText>, FetchError>`

**Tests** use a mock `Client` built from `tower::service_fn` that routes on the URI path:
- `pods_listed_from_namespace`
- `forbidden_pv_keeps_pvcs`: PVs return 403, so storage has the PVCs plus a `Forbidden { resource: "persistentvolumes", scope: "cluster" }` note.
- `unauthorized_maps_to_unauthorized`
- `timeout_maps_to_timeout`: use `tokio::time::pause`.
- `log_lines_sanitized_and_lossy`

---

### Task 5: Live state in App (panes, connection, generations, retention)

**Files:** modify `src/app.rs` and `src/model.rs`; create `src/state/pane.rs`.

**Interfaces:**
- `pub enum PaneStatus { Loading { since_ticks: u64 }, Ready, Failed(FetchError), Partial(FetchError) }`. Each list keeps its last good items when it fails; it is then marked stale.
- `pub enum Connection { Sample, Connecting, Connected { server_version: SafeText }, Disconnected { error: SafeText } }`
- `AppEvent::Data(DataEvent)`, where `DataEvent` is `ListsLoaded { generation, lists }`, `Version { generation, result }`, `LogLine { generation, line }`, `LogEnded { generation, reason }`, `Yaml { generation, target, result }` or `Namespaces(result)`.
- **Retention:** after a refresh, each cursor stays on the same name if it still exists, and otherwise clamps.
- **Disappearance:** if the selected resource disappears, `viewer_notice = Some("… is no longer present")` and logs stop.
- **Refresh timing:** auto-refresh every 40 ticks when no refresh is in flight. `r` forces one.
- **Generations:** a context or namespace switch increments `generation`, clears the data and goes to `Loading`.

**Tests:**
- `stale_generation_ignored`
- `selection_kept_by_name_after_refresh`
- `selected_pod_disappears_sets_notice_and_stops_logs`
- `forbidden_pane_keeps_others`
- `unauthorized_marks_disconnected`
- `auto_refresh_every_40_ticks_without_overlap`
- `failed_refresh_keeps_last_items_as_stale`

---

### Task 6: UI for live states

**Files:** modify `src/ui/header.rs`, `src/ui/lists.rs` and `src/ui/viewer/*`.

**What it renders:**
- **Header (Live):** `Kubeglass  ctx <ctx>  ns <ns>  ● Connected  api <version>  ⟳ every 10s`, or `◌ Connecting…`, or `● Disconnected · <error>` (err style). Sample mode is unchanged.
- **Panes:**
  - `Loading`: `⠋ loading pods… (Ns)`.
  - `Failed`, Forbidden: `✕ Forbidden (403)`, then `cannot list <resource> at the <scope> scope`, then `Needed (read-only): <resource>: get, list`.
  - `Failed`, other errors: `✕ <error>` plus `r retry`.
  - Stale data: the title gets ` · STALE`.
  - `Partial`: the rows, plus a footer line with the note.
- **Viewer:**
  - `viewer_notice` takes precedence: `⊘ Pod x is no longer present`.
  - YAML tab: `loading YAML…`, the YAML, or the error.
  - Logs: `● streaming` / `○ stream ended: <reason>`, with "(sample)" only in Sample mode.

**Tests** (render, using a constructed live `App`):
- `header_shows_real_context_and_version`
- `forbidden_pane_message`
- `loading_pane_message`
- `stale_title`
- `disappeared_notice`
- `disconnected_header`

---

### Task 7: Pickers, filter and refresh keys

**Files:** create `src/ui/picker.rs` and `src/state/picker.rs`; modify `app.rs`.

**Keys:**
- `x`: context picker (contexts from Task 2, with filter-as-you-type). Shows the hint, the server host, `current`, and the source line.
- `n`: namespace picker (from `FetchNamespaces`). If listing namespaces is forbidden, the typed text is accepted as the name.
- `Enter`: queues `SwitchTarget`. `Esc` closes the picker.
- `/`: filter the focused list by name substring. The filter shows in the pane title as `filter: <text>`; `Esc` clears it.
- `r`: queues `Refresh`.
- Pickers and the filter capture keys while open.

**Tests:**
- `x_opens_picker_and_filters`
- `enter_switches_target_and_bumps_generation`
- `namespace_picker_free_text_when_forbidden`
- `filter_narrows_rows_and_cursor_follows`
- `esc_clears_filter`
- `r_queues_refresh`

---

### Task 8: Runtime shell, signals, CLI

**Files:** create `src/runtime.rs`; modify `src/main.rs` and `src/cli.rs` (add `--demo`; `-A` → error "arrives in phase 3").

**Runtime:**
- A tokio multi-thread runtime.
- An input thread that does a blocking `crossterm::event::read` and sends to an mpsc channel.
- A `select!` over input, a 250ms tick, `Data` results, SIGTERM, SIGHUP and SIGINT. Signals set `app.quit`, so the terminal is restored.
- Commands are executed as spawned tasks:
  - log streams keep a `JoinHandle` that is aborted on `StopLogs` or `StartLogs`
  - `SwitchTarget` rebuilds the client.
- **Startup:** resolve the target. On error, render the header with the error and the hint `--demo`, and still exit cleanly on `q`.

**Tests:**
- `cli::demo_flag`
- `cli::all_namespaces_rejected_for_now`

**Manual:** the pty checks from phase 1 rerun with `--demo`. Also send SIGTERM and check that the alternate screen is left.

---

### Task 9: Real-cluster acceptance, docs, review

- Run `kglass` against `dev-cluster-01` / `team-namespace` (read-only) through the pty script. The header must show the real context, namespace and server version, the pod count must match `kubectl get pods --no-headers | wc -l`, and logs must stream for a running pod.
- Update the README, `keybindings.md` (x, n, /, r) and the design note.
- Get an independent code review, then fix what it finds, each fix with a test.
