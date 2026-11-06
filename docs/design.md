# Kubeglass (`kglass`) — Design Note

> **Status:** Design agreed in principle; implementation not started. Last updated 2026-11-06.
> **What it is:** an open-source Rust terminal UI for inspecting Kubernetes clusters. You pick a resource from a list and the main pane shows what it is, how it is doing, and what it relates to.
> **Name:** Kubeglass, command `kglass`. Sailors call a telescope a "glass"; the tool makes the cluster transparent. Free on crates.io and Homebrew (checked 2026-11-06). Formerly "Cluster Viewer", then briefly "Periscope" (taken).
> **Source material (read these too):**
> - Full product plan: [`../../cluster-viewer-project-plan.md`](../../cluster-viewer-project-plan.md), written under the working name "Cluster Viewer"
> - Visual design: [`../../Cluster Viewer TUI.pdf`](../../Cluster%20Viewer%20TUI.pdf), 3 pages: wide layout, narrow layout, edge states

This note is the working spec. Where it conflicts with the project plan, this note wins.

---

## 1. Intent

Debugging a cluster today means switching between `kubectl get`, `describe` and `logs`, and lining up pods, images, services and volumes in your head. Kubeglass keeps the resource lists on screen next to a large viewer, so the question "what is this, and what is it connected to?" is answered by selecting it.

Who it is for: developers, platform engineers and SREs who already have kubeconfig access. The first real users are the owner and the owner's RKE2 dev clusters (`dev-cluster-01/02`).

## 2. Releases

The project is split into two releases with a hard line between them.

### Release 1: Viewer (read-only). This note covers it.
- Never changes cluster state. Needs only `get`, `list` and `watch` RBAC plus `pods/log`.
- The `READ-ONLY` / `RO` badge in the header is a **mode indicator**, not a fixed label, so release 2 can switch it.

### Release 2: Actions on resources (needs its own design, not started)
- Create, edit, delete, scale and restart. Possibly port-forward and exec.
- Must follow the plan's safety rules (plan §9 "Future write actions", ADR-003):
  - show the exact target and what will change
  - require deliberate confirmation
  - offer a dry-run where the API supports it
  - keep read-only use possible
  - never turn a read action into an implicit write
- Release 1 must leave room for this. Cluster access goes behind a read-only interface, and writes are added later as a separate, gated module without rewriting the viewer.

### Not planned for either release (defaults, see open question Q1)
- Replacing `kubectl` or covering every API operation
- Cluster installation or provisioning
- A local database, telemetry, or a required account
- Image vulnerability scanning or registry credential management
- Full CRD coverage

> Note on the project plan: in plan §3 the "Creating, editing… / Replacing kubectl… / CRD" bullets sit under the **MVP goals** heading because a heading is missing. They are non-goals for release 1.

## 3. Release 1 goals

1. Discover and switch kubeconfig contexts and namespaces, including all namespaces where permissions allow.
2. Browse **Pods, Images** (built from pod specs), **Services, PVCs and PVs**.
3. Select with mouse click or keyboard. Every action must work with the keyboard alone.
4. A selected pod opens its **live logs**. Other resources open **details and related resources**.
5. Input never freezes while requests or log streams are running.
6. Each pane has clear empty, loading, permission-denied, disconnected, stale, deleted and error states.

**Anti-goals.** Release 1 counts as a failure even if it works, if it:
- shows stale data as if it were current
- implies a derived link (selector match, image use) proves reachability or health
- prints credentials
- lets cluster text send control sequences to the terminal
- has one failing request blank the whole UI

## 4. Screens (from the design PDF)

### 4.1 Wide layout (PDF page 1)
```
Header: Kubeglass · ctx <ctx> ▾ · ns <ns> ▾ · ● Connected · api vX.Y.Z · <distro> · ⟳ refreshed Ns ago · every 10s · [READ-ONLY]
Left column (stacked, numbered):           Right: viewer
  [0] Context   context / namespace / user   Tabs: Logs · Summary · Events · YAML · Containers     "Pod <name>"
  [1] Pods      NAME STATUS READY RST AGE    Toolbar: container ▸ api envoy · follow ● on · tail 500 · previous n/a · / search
  [2] Images    REPOSITORY TAG/DIGEST USED   Body: log lines, timestamp + level colouring
  [3] Services  NAME TYPE CLUSTER-IP PORTS EP
  [4] Storage   KIND NAME STATUS CAP MODE CLASS   Status bar: ● streaming · N lines buffered (cap 5000) · wrap on · timestamps on
Footer: ↑↓ navigate · enter select · tab focus viewer · 1-4 jump to list · / filter · n namespace · x context · r refresh · q quit · v0.1.0 · ? help
```
- Pod status shows a symbol and text, never colour alone: `●` Running, `✕` CrashLoopBackOff/Failed, `✓` Completed, `◌` Pending.
- Images list a digest as `@sha256:9f2c…e81a`. `USED` is the number of containers using the image.
- Services `EP` is the ready endpoint count, highlighted when it is 0.
- Storage shows PVCs and PVs in one table, with a `KIND` column.

### 4.2 Narrow layout (PDF page 2, about 92×48)
- Compact header: `CV ctx … ns … ● Connected [RO]`. In Kubeglass the short form is `KG`.
- Lists become a tab strip: `[1] Pods 6 · [2] Images 7 · [3] Services 5 · [4] Storage 5`. One list is shown at a time with a `filter:` label.
- The viewer goes below the list. `z` zooms the viewer to full screen.
- Example: Service details show a warning when there are no ready endpoints. Matching pods appear as `→ billing-0 CrashLoopBackOff [derived]`, followed by "Selector matches show configuration, not network reachability." Enter jumps to the pod.

### 4.3 Edge states (PDF page 3)
| State | Behaviour |
|---|---|
| Context/namespace picker | `x` / `n` open it. Type to filter. Shows the current context and the provider/region hint. A broken context shows its reason, e.g. "exec plugin … not found". Shows the source (`~/.kube/config + $KUBECONFIG`). Switching clears stale data. |
| Loading & partial data | Each list loads on its own, with a spinner and elapsed time on the loading part. Timeout 15s. `r` retries. Shows partial results (PVCs ready, PVs still loading). |
| Permission denied | Shown per resource: `Forbidden (403)`, the user, the verb, the resource and the scope, and the exact rule needed. Other panes keep working. |
| Disconnected / stale | Header shows `● Disconnected`, the reconnect countdown, the attempt number, back-off 2s→30s and the last error. Lists are marked **STALE** with their age. The log stream pauses and resumes without replaying. |
| Selected resource disappeared | The row stays as a ghost row. The viewer shows "no longer present", how long ago it was deleted and why when known, and the last buffered logs as a snapshot. Enter selects a sibling, Esc dismisses. |
| Startup auth failure | A blocking screen before any pane loads. Shows the plugin error (e.g. token expired) and the fix hint. `r` retries, `x` picks another context, `q` quits. Credentials are always redacted. |

### 4.4 Viewer tabs per resource
| Resource | Default tab | Other tabs |
|---|---|---|
| Pod | Logs | Summary, Events, YAML, Containers |
| Image | Details | Workloads using image |
| Service | Details | Endpoints, Events, YAML |
| PVC | Details | Mounted by, Events, YAML |
| PV | Details | Claim, Storage details, YAML |

Only tabs that have something to show are displayed.

### 4.5 Keys
`↑↓`/`PgUp`/`PgDn` move · `Enter` select / follow a link · `Tab`/`Shift+Tab` focus · `←→` viewer tabs · `1-4` jump to list · `0` context · `/` filter (list) or search (logs) · `Esc` clear/close · `n` namespace · `x` context · `r` refresh/retry · `z` zoom viewer · `f` follow toggle (my call) · `?` help · `q`/`Ctrl+C` quit.
Mouse: click a row, a pane or a tab; the wheel scrolls. Mouse is optional and must never be required.

## 5. Architecture (decided)

- **One Rust binary.** `ratatui` + `crossterm` for the UI, `kube` (kube-rs) + `k8s-openapi` for the API, `tokio` for async, `serde`/`serde_yaml` for YAML, `tracing` writing to a file only (never to the TUI screen), `clap` for the CLI.
- **Event loop.** One loop owns `App` state. Input, timers and resize events, plus results from background tasks, all arrive as typed `AppEvent`s on a channel. Background tasks never change state directly.
- **Refresh.** Lists poll on an interval (default 10s) and on `r`. Release 1 has no watch streams per resource. Every request has a timeout, and each pane has its own loading/error state.
- **Logs.** One active stream, for the selected pod and container. Selecting something else cancels it. A ring buffer holds the lines (default cap 5000). Follow can be paused.
- **Safety.** All cluster text is sanitised before rendering: control characters and escape sequences are stripped. Secrets are never fetched. Credentials never appear in the UI, errors or logs.
- **Read-only seam.** A `ClusterReader` trait (or a module with the same role) exposes only read operations. Release 2 adds a separate writer behind a mode switch.
- **Testable without a cluster.** The UI and state run against a `FakeCluster` that serves the sample data from the design PDF. That powers the phase 1 prototype and the unit tests.

### Module layout
```
kglass/
├── Cargo.toml, README.md, LICENSE, CONTRIBUTING.md, SECURITY.md, CODE_OF_CONDUCT.md
├── docs/ design.md (this file), architecture.md, keybindings.md, rbac.md
└── src/
    ├── main.rs        CLI, terminal setup/restore (incl. on panic)
    ├── app.rs         App state + update(AppEvent)
    ├── event.rs       AppEvent, input/tick/async sources
    ├── config.rs
    ├── kube/          client.rs, pods.rs, logs.rs, services.rs, storage.rs, events.rs, images.rs, relationships.rs, fake.rs
    ├── state/         lists, selection (incl. ghost rows), log_buffer
    └── ui/            layout.rs (wide/narrow), header.rs, lists.rs, viewer/*.rs, picker.rs, help.rs, theme.rs, sanitize.rs
```

## 6. Build phases

| Phase | What | Done when |
|---|---|---|
| **0. Setup** ✅ partly | Name chosen. Rust 1.99.1 installed. | `cargo` works. Name availability checked. |
| **1. Static prototype** | Scaffold the crate, terminal init/restore, event loop, wide and narrow layouts, all four lists, viewer tabs, keyboard and mouse, sample data from the PDF (`FakeCluster`), no-colour mode. | Matches the PDF pages 1–2 on sample data, resizes cleanly, exits cleanly even on panic. |
| **2. Real lists** | kubeconfig contexts, namespaces, picker, and the Pods, Services, PVC and PV lists. Images built from pods. Polling refresh, filter, selection kept across refresh, per-pane loading/403/timeout states, disconnect and stale handling. | Browses the dev RKE2 clusters without blocking input. |
| **3. Viewer & logs** | Log streaming (container picker, follow, tail, previous, search, buffer cap). Summary, YAML, Events and Containers tabs. Service→pods and PVC↔PV↔pod links, labelled `[derived]`. Ghost rows. Startup auth screen. | Every MVP resource shows useful details. Switching pods cancels the old stream. |
| **4. Hardening & beta** | Test with read-only RBAC, test terminals (iTerm2, Terminal.app, a Linux terminal), README with GIFs, `docs/rbac.md`, CI (fmt, clippy, test), release binaries. | Install and first-run steps can be reproduced. Limitations are documented. |
| **5. Release 2 design** | A separate design note for write actions. | Approved before any write code is written. |

Testing follows plan §12: unit tests for row formatting, image dedup, relationships, the log buffer, state transitions and truncation; integration tests against fake and local clusters; the manual checklist.

## 7. Decisions log
| # | Decision | By |
|---|---|---|
| D1 | Name: **Kubeglass**, command and crate `kglass` (Periscope was taken) | owner |
| D2 | Release 1 read-only; writes are release 2 with their own design | owner |
| D3 | Rust + ratatui/crossterm + kube-rs + tokio (plan ADR-001/002) | owner (plan) |
| D4 | Start with a static prototype on sample data (plan §18) | owner (plan) |
| D5 | Polling refresh every 10s, request timeout 15s, log cap 5000, tail 500 (values from the design) | my call |
| D6 | Pods always open on Logs. If the container never started, show the reason and the Events in the viewer instead of an empty log | my call |
| D7 | Config: CLI flags (`--context`, `--namespace`, `--kubeconfig`) in phase 1, an optional TOML file in phase 4. Flags override the file | my call |
| D8 | YAML is a viewer tab, not a separate action | my call (follows the design) |
| D9 | Target macOS and Linux first; Windows later | my call |
| D10 | License **Apache-2.0 OR MIT** (dual, the Rust norm) | my call, owner to confirm |
| D11 | Dev-cluster access is a **static bearer token through the Rancher proxy** (`https://<rancher-host>/k8s/clusters/<id>`), not an exec plugin. kube-rs handles this natively. Log streaming must be checked through the Rancher proxy early in phase 3. The auth screen's fix hint for an expired or invalid token says "regenerate the kubeconfig/token in Rancher". | found in kubeconfig, 2026-11-06 |
| D12 | The acceptance target is a **local kind/k3d cluster** seeded with the design's failure cases: a CrashLoop pod, a Pending pod, a Pending PVC, a service with 0 endpoints, a released PV, and a read-only ServiceAccount that triggers 403 on PVs. The the dev cluster clusters are the real-world check when the VPN is up. | my call |
| D13 | Scale: requests use `limit=500` with `continue` paging, and tables render only visible rows. Above 2000 items per list, show a "truncated · narrow with namespace or filter" banner. Refresh skips a list whose previous request is still in flight. | my call |
| D15 | **Pick up the context and namespace the same way kubectl does**, so `kubectx`/`kubens` users and plain-env users get the same result. Kubeconfig: `--kubeconfig` > `$KUBECONFIG` (colon-separated list, merged) > `~/.kube/config`. Context: `--context` > `current-context`. Namespace: `-n/--namespace` > the context's `namespace` (what `kubens` sets) > `default`. All namespaces: `-A`. Same flag names as kubectl. (`kube::Config::infer` / `Kubeconfig::read` already follow these rules.) | owner (intent), my call (details) |
| D14 | The context picker **only reads kubeconfig**, never contacting each context: it shows the cluster name, the server host, and a `rancher`/`kind`/`eks` hint guessed from the URL. A context is contacted only after the user picks it, so no credential plugins run for clusters the user didn't choose. | my call |

## 8. Open questions
- **Q1.** Do the "not planned" items in §2 stay permanent non-goals, or move into release 2? Default: permanent.
- ~~Q2~~ **Answered:** `kglass` is free on crates.io and Homebrew. Check the GitHub repo name when creating it.
- **Q3.** Which Kubernetes version range does it support? The RKE2 dev clusters were not reachable on 2026-11-06 (i/o timeout, probably VPN). Check the server version, then choose the `k8s-openapi` feature (likely the lowest supported version that covers the dev clusters).
- **Q4.** Should the "Images" list include images from workload templates (Deployments, StatefulSets) or only running pods? Default: pods only in release 1.
- ~~Q5~~ **Answered (owner):** show cluster data **as-is, like kubectl**. No redaction of env values or annotations. Kubeconfig credentials are still never shown, and Secret objects are still not fetched.
- ~~Q6~~ **Answered (owner):** this is the owner's own project and works with any cluster. work clusters are just one place to test. Keep the public repo free of internal cluster names: use generic example names (`dev-east`, `payments`) in docs and fixtures.
- **Q7.** When the user switches context or namespace inside the tool, should it **write that back to kubeconfig** (like `kubectx`/`kubens`, so later `kubectl` calls follow) or change it **for this session only** (like `kubectl --context/-n`)? Writing back touches a local file, not the cluster, so it doesn't break "read-only".
- ~~Q8~~ **Answered:** Kubeglass / `kglass` (§10).

## 10. Naming
**Decided: Kubeglass / `kglass`** (owner, 2026-11-06). "Periscope" was taken on crates.io and Homebrew. Candidates considered:

| Name | Command | Meaning | crates.io / brew |
|---|---|---|---|
| Kubeglass | `kglass` | "glass" is sailors' word for a telescope, and the tool makes the cluster transparent | free / free |
| Kubesight | `ksight` | sight into the cluster | free / free |
| Tidewatch | `tidewatch` | watching the tide of pods come and go | free / free |
| Fathom | `kfathom` | to fathom = measure the depth / understand (`fathom` crate is taken) | free / free |

## 9. Where we are / next step
- 2026-11-06: plan and design reviewed, name chosen, Rust installed, this note written.
- 2026-11-06: builder check done (D11–D14). Owner answered Q5 (show data as-is like kubectl), Q6 (own project, any cluster) and Q8 (name **Kubeglass / `kglass`**). Q7 (whether in-app context/namespace switches are saved to kubeconfig) is still open.
- 2026-11-06: **Phase 0 done.** `kglass` is free on crates.io, Homebrew and npm, with no notable GitHub project.
- 2026-11-06: **Phase 1 done** (static prototype, [`plan-phase1.md`](plan-phase1.md), 7 tasks, one commit each).
  - **Delivered:** wide and narrow layouts matching PDF pages 1–2; all four lists; every viewer tab per resource; keyboard and mouse; simulated log streaming; no-colour mode; a "too small" screen.
  - **Tests:** 93, all passing, with fmt and clippy `-D warnings` clean.
  - **Real-terminal checks** (through a pty): the alternate screen and mouse reporting are restored on `q`, `Ctrl+C` and panic; resize to narrow and to too-small both work; `NO_COLOR` emits no colour codes.
  - **Independent code review:** every finding was fixed with a test. Pausing follow now keeps the view. Zoom focuses the viewer, and list keys leave zoom. Labels, selectors and image fields are now `SafeText`. The sanitizer strips bidi and zero-width characters. Re-selecting a row keeps viewer state. Help fits 60×16. Width math no longer overflows. Pending-pod text scrolls. Ctrl/Alt key combos are ignored.
  - **Deviations from the plan:**
    - Log lines are stored as `SafeText`, so they are sanitized on ingest.
    - Pods stream logs whatever tab is open.
    - `z` moves focus to the viewer.
    - Ages show as `2h05m`, as in the PDF.
- **Carried into phase 2:**
  - Restore the terminal on SIGTERM/SIGHUP (tokio signal handling arrives with tokio).
  - Answer Q7.
  - `--kubeconfig` is accepted but not used yet.
- **Next:** write `docs/plan-phase2.md`, covering real kubeconfig loading per D15, the context/namespace pickers, real lists with loading/403/stale states, filter and refresh.
