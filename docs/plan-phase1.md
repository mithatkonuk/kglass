# Kubeglass Phase 1 — Static Prototype Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A `kglass` binary that renders the design's wide and narrow layouts from built-in sample data. Every list, viewer tab, key and mouse action of the prototype works. No Kubernetes calls yet.

**Architecture:** A library crate (`src/lib.rs`) holds all logic. `src/main.rs` only parses the CLI, sets up and restores the terminal, and pumps events. All state lives in `App` and changes only through `App::update(AppEvent)`, which is pure and has no I/O, so tests drive it directly. Phase 2 swaps the event source for a tokio channel and adds variants, without touching `update`'s callers. Rendering is a function `ui::draw(&mut Frame, &mut App)`. While it draws, it records clickable regions in `app.hits`, which mouse handling reads back. Every string that comes from the cluster is wrapped in `SafeText`, which can only be built through `sanitize`. That makes "cluster text can't send terminal commands" a guarantee of the type system.

**Tech Stack:**
- Rust 2024 edition
- `ratatui 0.30.2` (uses its re-exported `ratatui::crossterm`; no direct crossterm dependency)
- `clap 4.6` with derive
- `color-eyre 0.6.5`
- `unicode-width 0.2.3`
- No tokio or kube in phase 1. They arrive in phase 2. This is my call (YAGNI).

**Spec:** [`docs/design.md`](design.md). Read §3 (goals and anti-goals), §4 (screens) and §5 (architecture). The screens to match are pages 1 and 2 of `../../Cluster Viewer TUI.pdf`.

## Global Constraints

- Minimum supported Rust version: 1.88 (required by ratatui 0.30). Crate and binary are named `kglass`. In the UI, the full name is `Kubeglass` and the short header label is `KG`. Repo root is `/Users/mko/realm/personal/kglass`.
- Release 1 is read-only. The header always shows a mode badge: `READ-ONLY` in the wide layout, `RO` in the narrow one.
- Status is never shown by colour alone. Pod symbols: `●` Running, `✕` CrashLoopBackOff/Failed, `✓` Completed, `◌` Pending. A text label always goes next to the symbol.
- No-colour mode is on when `--no-color` is passed or the `NO_COLOR` env var is set and non-empty. It uses bold and reverse video only.
- Every cluster-originated string is a `SafeText`. Never build ratatui `Span`s from cluster data without going through `SafeText`.
- Default values (design D5):
  - log buffer cap `5000`
  - log tail `500`
  - refresh label `every 10s`
  - simulated log interval `437ms`
  - tick `250ms`
- Layout breakpoints (my call):
  - **Wide** when the terminal is at least `120` columns wide and `30` rows high.
  - **TooSmall** when it is under `60` columns or under `16` rows.
  - **Narrow** otherwise.
- CLI flag names match kubectl: `--kubeconfig`, `--context`, `-n/--namespace`, `-A/--all-namespaces`. In phase 1 these only override the labels shown in the header.
- Public repo hygiene: use only generic example names from the PDF (`dev-east`, `payments`, `registry.acme.io/...`). No internal cluster names.
- Quality gate for every task: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` all pass.

## Review Focus

1. **Tiny or degenerate terminal sizes**, e.g. 40×10, 1×1, or a resize to 0 rows. The app must not panic. It renders `Terminal too small — need at least 60×16` instead. The test is in Task 5.
2. **Hostile cluster text**: ANSI/OSC escape sequences, `\r`, BEL, very long names, wide CJK characters or emoji in pod names. Escapes are stripped. Text is truncated by display width with `…` and never overflows its column. Tests are in Task 1 and Task 5.
3. **Empty lists**, e.g. a namespace with zero services. Cursor moves, Enter and clicks are no-ops. The pane shows `No services in payments`. The viewer shows `Nothing selected` if the selection is empty. Tests are in Task 3 and Task 5.
4. **Log buffer at its cap while not following.** The view stays on the same lines as older ones are dropped, and the scroll position never points past the end. The test is in Task 3.
5. **Resize while zoomed or while a hidden list has focus**, e.g. going from wide to narrow with focus on `Storage`. Narrow mode shows the focused list's tab. Zoom survives the resize. No index points outside the visible panes. The test is in Task 6.

---

## File Structure

```
kglass/
├── Cargo.toml                 deps + [lints]
├── LICENSE-MIT, LICENSE-APACHE, README.md, .gitignore
├── docs/ design.md, plan-phase1.md, keybindings.md (Task 7)
├── src/
│   ├── main.rs                CLI → terminal init/restore → event pump (Task 7)
│   ├── lib.rs                 pub mod declarations
│   ├── cli.rs                 clap Args (Task 7)
│   ├── text.rs                SafeText, sanitize, truncate, shorten_digest (Task 1)
│   ├── model.rs               domain types + formatting helpers (Task 2)
│   ├── derive.rs              images_in_use, pods_matching (Task 2)
│   ├── fake.rs                sample_snapshot, FakeLogs, sample_yaml, sample events (Task 2)
│   ├── state/
│   │   ├── mod.rs
│   │   ├── list_cursor.rs     ListCursor (Task 3)
│   │   └── log_buffer.rs      LogBuffer (Task 3)
│   ├── app.rs                 App, AppEvent, Focus, ViewerTab, tabs_for, update (Task 4)
│   └── ui/
│       ├── mod.rs             draw(), TooSmall screen (Task 5)
│       ├── theme.rs           Theme::color()/plain() (Task 5)
│       ├── layout.rs          LayoutMode, Regions, regions() (Task 5)
│       ├── hits.rs            HitMap, Hit (Task 6)
│       ├── header.rs, footer.rs, lists.rs, help.rs (Task 5)
│       └── viewer/ mod.rs, pod.rs, image.rs, service.rs, storage.rs (Task 6)
└── tests/
    ├── support/mod.rs         render_to_string(app, w, h) helper (Task 5)
    ├── render_wide.rs, render_narrow.rs (Task 5/6)
    └── mouse.rs (Task 6)
```

---

### Task 1: Scaffold the crate + safe text utilities

**Files:**
- Create: `Cargo.toml`, `src/lib.rs`, `src/main.rs` (stub `fn main() {}`), `src/text.rs`, `LICENSE-MIT`, `LICENSE-APACHE`, `README.md` (name, one-line pitch, "status: phase 1 prototype, sample data only"), `.gitignore` (`/target`)
- Test: unit tests inside `src/text.rs`

**Interfaces:**
- Produces:
  - `pub struct SafeText(String)`, which derives `Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default`.
    - `SafeText::new(raw: &str) -> SafeText` (runs `sanitize`)
    - `SafeText::as_str(&self) -> &str`
    - `impl Display`, `impl From<&str>`, `impl From<String>` (both sanitize). There is no other constructor.
  - `pub fn sanitize(raw: &str) -> String`
  - `pub fn truncate(s: &str, max_width: usize) -> String`
  - `pub fn shorten_digest(digest: &str) -> String`

- [ ] **Step 1: Set up the repo.** `cd /Users/mko/realm/personal/kglass && git init && cargo init --lib --name kglass`, then add `src/main.rs`.
  - Cargo.toml: `edition = "2024"`, `rust-version = "1.85"`, `license = "Apache-2.0 OR MIT"`.
  - `[dependencies]`: `ratatui = "0.30.2"`, `clap = { version = "4.6", features = ["derive"] }`, `color-eyre = "0.6.5"`, `unicode-width = "0.2.3"`.
  - `[lints.rust] unsafe_code = "forbid"`.
- [ ] **Step 2: Write the failing tests** in `src/text.rs`:

```rust
#[test] fn strips_csi_and_osc_sequences() {
    assert_eq!(sanitize("\x1b[31mred\x1b[0m"), "red");
    assert_eq!(sanitize("a\x1b]0;evil title\x07b"), "ab");
    assert_eq!(sanitize("a\x1b]8;;http://x\x1b\\link\x1b]8;;\x1b\\b"), "alinkb");
}
#[test] fn replaces_control_chars() {
    assert_eq!(sanitize("a\tb"), "a b");        // tab -> one space
    assert_eq!(sanitize("a\rb\x07c\x7fd"), "abcd"); // CR, BEL, DEL dropped
    assert_eq!(sanitize("a\nb"), "a b");        // newline -> space (single-line contexts)
    assert_eq!(sanitize("a\u{9b}31mb"), "a31mb"); // C1 CSI dropped
}
#[test] fn safetext_only_via_sanitize() {
    assert_eq!(SafeText::from("x\x1b[2Jy").as_str(), "xy");
}
#[test] fn truncate_by_display_width() {
    assert_eq!(truncate("api-7c9d8f-x2k4", 20), "api-7c9d8f-x2k4");
    assert_eq!(truncate("api-7c9d8f-x2k4", 8), "api-7c9…");
    assert_eq!(truncate("日本語ポッド", 7), "日本語…");   // each CJK char is width 2
    assert_eq!(truncate("abc", 0), "");
    assert_eq!(truncate("abc", 1), "…");
}
#[test] fn shortens_sha256_digest() {
    let d = "sha256:9f2c4b7e1d0a3c5f6e7d8c9b0a1f2e3d4c5b6a7980f1e2d3c4b5a69788e81a";
    assert_eq!(shorten_digest(d), "sha256:9f2c…e81a");
    assert_eq!(shorten_digest("v2.14.0"), "v2.14.0");          // not a digest: unchanged
}
```

- [ ] **Step 3:** Run `cargo test text::`. Expected: compile FAIL because nothing is defined yet.
- [ ] **Step 4: Implement the functions.** `sanitize` is a small state machine over `chars()`:
  - `ESC [` … final byte `0x40..=0x7E` → dropped
  - `ESC ]` … `BEL` or `ESC \` → dropped
  - any other `ESC x` → both dropped
  - `\t` and `\n` → space
  - other C0, DEL and C1 (`0x80..=0x9F`) → dropped

  `truncate` uses `unicode_width::UnicodeWidthChar` and reserves one column for `…`. `shorten_digest` applies only when the input starts with `sha256:` and has more than 16 hex characters. It keeps the first 4 and last 4 hex characters.
- [ ] **Step 5:** Run `cargo test`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check`. Expected: PASS.
- [ ] **Step 6: Commit** with message `feat: scaffold kglass crate with SafeText sanitizing`.

---

### Task 2: Domain model, sample data, derived relationships

**Files:**
- Create: `src/model.rs`, `src/derive.rs`, `src/fake.rs`
- Test: unit tests in each file

**Interfaces:**
- Consumes: `SafeText`, `shorten_digest` (Task 1)
- Produces (in `model.rs`):
  - `pub enum ResourceKind { Pods, Images, Services, Storage }` with:
    - `const ALL: [ResourceKind; 4]`
    - `fn number(self) -> u8` (1..=4)
    - `fn title(self) -> &'static str` ("Pods", "Images", "Services", "Storage")
    - `fn from_number(n: u8) -> Option<Self>`
  - `pub enum PodStatus { Running, Pending, Completed, Failed, CrashLoopBackOff }` with `fn symbol(self) -> &'static str` and `fn label(self) -> &'static str`
  - `pub struct Container { pub name: SafeText, pub image: SafeText, pub ready: bool, pub restarts: u32, pub init: bool, pub waiting_reason: Option<SafeText> }`
  - `pub struct Pod { pub name: SafeText, pub namespace: SafeText, pub status: PodStatus, pub labels: BTreeMap<String, String>, pub containers: Vec<Container>, pub age: Duration, pub node: SafeText, pub pod_ip: SafeText, pub owner: Option<SafeText> }`
    - `fn ready(&self) -> (u32, u32)` counts non-init containers only
    - `fn restarts(&self) -> u32` sums non-init containers
  - `pub enum ServiceType { ClusterIP, NodePort, LoadBalancer, ExternalName }`
  - `pub struct ServicePort { pub port: u16, pub target_port: u16, pub node_port: Option<u16>, pub protocol: SafeText }`
  - `pub struct Service { pub name: SafeText, pub namespace: SafeText, pub service_type: ServiceType, pub cluster_ip: SafeText, pub ports: Vec<ServicePort>, pub selector: BTreeMap<String, String>, pub ready_endpoints: u32 }`
  - `pub enum StorageKind { Pvc, Pv }`
  - `pub struct StorageItem { pub kind: StorageKind, pub name: SafeText, pub status: SafeText, pub capacity: SafeText, pub access_modes: SafeText, pub storage_class: SafeText, pub bound_to: Option<SafeText>, pub mounted_by: Vec<SafeText> }`
  - `pub struct ImageRef { pub repository: String, pub tag: Option<String>, pub digest: Option<String> }`
    - `fn parse(raw: &str) -> ImageRef`
    - `fn tag_or_digest(&self) -> String`: the tag, or `@` + `shorten_digest`, or `latest` when neither is present
  - `pub struct ImageUsage { pub raw: SafeText, pub image: ImageRef, pub users: Vec<(SafeText, SafeText)> }`, where each user is (pod name, container name)
  - `pub enum EventType { Normal, Warning }`
  - `pub struct KubeEvent { pub object: ResourceRef, pub event_type: EventType, pub reason: SafeText, pub message: SafeText, pub age: Duration, pub count: u32 }`
  - `pub enum ResourceRef { Pod(SafeText), Image(SafeText), Service(SafeText), Pvc(SafeText), Pv(SafeText) }`, which derives `Clone, Debug, PartialEq, Eq`
  - `pub struct ClusterSnapshot { pub context: SafeText, pub namespace: SafeText, pub user: SafeText, pub api_version: SafeText, pub distro: SafeText, pub pods: Vec<Pod>, pub services: Vec<Service>, pub storage: Vec<StorageItem>, pub events: Vec<KubeEvent> }`
  - `pub fn format_age(d: Duration) -> String`
  - `pub fn format_ports(ports: &[ServicePort]) -> String`
- Produces (in `derive.rs`):
  - `pub fn images_in_use(pods: &[Pod]) -> Vec<ImageUsage>`
  - `pub fn pods_matching<'a>(selector: &BTreeMap<String, String>, pods: &'a [Pod]) -> Vec<&'a Pod>`
- Produces (in `fake.rs`):
  - `pub fn sample_snapshot() -> ClusterSnapshot`
  - `pub fn sample_yaml(r: &ResourceRef, snap: &ClusterSnapshot) -> String`
  - `pub struct FakeLogs` with:
    - `pub fn new() -> Self`
    - `pub fn initial(&self, pod: &Pod, container: &str, tail: usize) -> Vec<String>`
    - `pub fn next_line(&mut self, pod: &Pod, container: &str) -> Option<String>`

**Sample data:** copy it exactly from PDF page 1.

- **Header:** context `dev-east`, namespace `payments`, user `sso-readonly`, api `v1.31.4`, distro `eks`.
- **Pods.** Containers are written as name:image; image repos are under `registry.acme.io/payments/`.

  | Pod | Status | Containers | Restarts | Age |
  |---|---|---|---|---|
  | `api-7c9d8f-x2k4` | Running | `api`:`api:v2.14.0` + `envoy`:`envoyproxy/envoy:v1.31.2`, both ready | 0 | 3h12m |
  | `api-7c9d8f-q8n1` | Running | same as above | 1 | 3h12m |
  | `worker-65b9c6-p2m8` | Running | `worker`:`worker:v4.3.1` | 0 | 1d4h |
  | `billing-0` | CrashLoopBackOff | init `busybox:1.36` + `billing`:`billing@sha256:9f2c4b7e1d0a3c5f6e7d8c9b0a1f2e3d4c5b6a7980f1e2d3c4b5a69788e81a`, not ready, waiting reason `CrashLoopBackOff` | 17 | 2h05m |
  | `migrate-28841-zt5` | Completed | `migrate:v2.14.0` | 0 | 14m |
  | `ledger-sync-5f7d-h2c` | Pending | `ledger-sync:v0.9.2`, waiting reason `ContainerCreating` | 0 | 2m |

  Labels: `app=<prefix>` (`api`, `worker`, `billing`, `migrate`, `ledger-sync`).
- **Services:**

  | Service | Type | Cluster IP | Ports | Selector | Ready endpoints |
  |---|---|---|---|---|---|
  | `api` | ClusterIP | 10.96.14.22 | 80→8080 TCP | app=api | 2 |
  | `api-grpc` | ClusterIP | 10.96.14.40 | 9090 | app=api | 2 |
  | `billing` | ClusterIP | 10.96.31.7 | 8443 | app=billing | 0 |
  | `worker-metrics` | ClusterIP | `None` | 9100 | app=worker | 1 |
  | `payments-lb` | LoadBalancer | 10.96.2.18 | 443, nodePort 31443 | app=api | 2 |

- **Storage:**

  | Kind | Name | Status | Capacity | Access | Class | Bound to | Mounted by |
  |---|---|---|---|---|---|---|---|
  | PVC | `data-billing-0` | Bound | 20Gi | RWO | gp3 | `pvc-3e1a9c52-0c09` | `billing-0` |
  | PVC | `ledger-cache` | Pending | 5Gi | RWO | gp3 | — | `ledger-sync-5f7d-h2c` |
  | PVC | `uploads` | Bound | 50Gi | RWX | efs | — | — |
  | PV | `pvc-3e1a9c52-0c09` | Bound | 20Gi | RWO | gp3 | `payments/data-billing-0` | — |
  | PV | `pvc-77b2d4f1-141f` | Released | 50Gi | RWO | gp3 | — | — |

- **Events:** at least `billing-0` Warning `BackOff` "Back-off restarting failed container billing" ×17, and `ledger-sync-5f7d-h2c` Normal `Scheduled`, plus Warning `FailedScheduling`-style "waiting for PVC ledger-cache".

- [ ] **Step 1: Write the failing tests.**

```rust
// model.rs
#[test] fn ages() { assert_eq!(format_age(secs(3*3600+12*60)), "3h12m"); assert_eq!(format_age(secs(28*3600)), "1d4h");
                   assert_eq!(format_age(secs(14*60)), "14m"); assert_eq!(format_age(secs(42)), "42s"); }
#[test] fn ports() {
    assert_eq!(format_ports(&[port(80, 8080, None)]), "80→8080/TCP");
    assert_eq!(format_ports(&[port(9090, 9090, None)]), "9090/TCP");
    assert_eq!(format_ports(&[port(443, 443, Some(31443))]), "443:31443/TCP");
    assert_eq!(format_ports(&[port(80, 80, None), port(443, 443, None)]), "80/TCP,443/TCP");
}
#[test] fn image_refs() {
    let r = ImageRef::parse("registry.acme.io:5000/payments/api:v2.14.0");
    assert_eq!((r.repository.as_str(), r.tag.as_deref()), ("registry.acme.io:5000/payments/api", Some("v2.14.0")));
    assert_eq!(ImageRef::parse("busybox").tag_or_digest(), "latest");
    assert_eq!(ImageRef::parse("registry.acme.io/payments/billing@sha256:9f2c4b7e1d0a3c5f6e7d8c9b0a1f2e3d4c5b6a7980f1e2d3c4b5a69788e81a").tag_or_digest(), "@sha256:9f2c…e81a");
}
#[test] fn pod_ready_ignores_init() { let s = sample_snapshot(); let b = s.pods.iter().find(|p| p.name.as_str()=="billing-0").unwrap();
    assert_eq!(b.ready(), (0, 1)); assert_eq!(b.restarts(), 17); }
// derive.rs
#[test] fn images_dedup_and_order() {
    let imgs = images_in_use(&sample_snapshot().pods);
    let rows: Vec<(String, usize)> = imgs.iter().map(|i| (i.image.repository.clone(), i.users.len())).collect();
    assert_eq!(rows, vec![
        ("envoyproxy/envoy".into(), 2), ("registry.acme.io/payments/api".into(), 2),
        ("busybox".into(), 1), ("registry.acme.io/payments/billing".into(), 1),
        ("registry.acme.io/payments/ledger-sync".into(), 1), ("registry.acme.io/payments/migrate".into(), 1),
        ("registry.acme.io/payments/worker".into(), 1)]);
}
#[test] fn selector_matching() {
    let s = sample_snapshot();
    let billing = s.services.iter().find(|x| x.name.as_str()=="billing").unwrap();
    assert_eq!(pods_matching(&billing.selector, &s.pods).iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["billing-0"]);
    assert!(pods_matching(&BTreeMap::new(), &s.pods).is_empty()); // empty selector matches nothing
}
// fake.rs
#[test] fn fake_logs_follow_pdf_timeline() {
    let s = sample_snapshot(); let api = &s.pods[0];
    let first = FakeLogs::new().initial(api, "api", 500);
    assert_eq!(first.len(), 34);
    assert!(first[0].starts_with("2026-11-06 12:04:11.000 INFO"));
    assert!(first[1].starts_with("2026-11-06 12:04:11.437"));
}
#[test] fn pending_pod_has_no_logs() { let s = sample_snapshot(); let p = &s.pods[5];
    assert!(FakeLogs::new().initial(p, "ledger-sync", 500).is_empty()); assert!(FakeLogs::new().next_line(p, "ledger-sync").is_none()); }
```

- [ ] **Step 2:** Run `cargo test model:: derive:: fake::`. Expected: compile FAIL.
- [ ] **Step 3: Implement the model and derive functions.**
  - `images_in_use` includes init containers.
  - It sorts by users descending, then repository ascending.
  - `ImageRef::parse` splits a digest off at `@`. It splits a tag off at the last `:` only if that `:` comes after the last `/`, so registry ports are handled.
- [ ] **Step 4: Implement `fake.rs`.**
  - Log lines cycle through the PDF's templates (`INFO http`, `DEBUG db`, `INFO events`, `WARN ledger`, and one `ERROR ledger` every 23rd line).
  - Timestamps start at `2026-11-06 12:04:11.000` and step by 437ms per line.
  - Lines use the format `{ts} {LEVEL:<5} {source:<7} {message}`.
  - `initial` returns `min(tail, 34)` lines.
  - Behaviour by pod status:
    - Completed: `initial` lines only, and `next_line` returns `None`.
    - Pending: nothing.
    - CrashLoopBackOff: 5 lines ending in a `panic`/`exit 1`, then `None`.
  - `sample_yaml` hand-builds a plausible manifest from the model. Real serialization comes in phase 2.
- [ ] **Step 5:** Run the quality gate. Expected: PASS.
- [ ] **Step 6: Commit** with message `feat: domain model, sample cluster data, derived images and selectors`.

---

### Task 3: List cursor and log buffer state

**Files:**
- Create: `src/state/mod.rs`, `src/state/list_cursor.rs`, `src/state/log_buffer.rs`

**Interfaces:**
- Produces:
  - `pub struct ListCursor`, which derives `Default`:
    - `fn set_len(&mut self, len: usize)` clamps the cursor
    - `fn len(&self) -> usize`
    - `fn selected(&self) -> Option<usize>` (`None` when the list is empty)
    - `fn move_by(&mut self, delta: isize)` saturates at both ends
    - `fn select(&mut self, idx: usize)` clamps
    - `fn offset(&self) -> usize`
    - `fn scroll_into_view(&mut self, viewport_rows: usize)`
  - `pub struct LogBuffer`:
    - `fn new(cap: usize) -> Self`
    - `fn push(&mut self, line: String)`
    - `fn extend(&mut self, lines: impl IntoIterator<Item = String>)`
    - `fn clear(&mut self)`
    - `fn len(&self) -> usize`
    - `fn cap(&self) -> usize`
    - `fn follow(&self) -> bool`
    - `fn set_follow(&mut self, on: bool)`
    - `fn scroll_by(&mut self, delta: isize, viewport: usize)`: scrolling up (`delta < 0`) turns follow off
    - `fn top(&self, viewport: usize) -> usize`: the index of the first visible line
    - `fn visible(&self, viewport: usize) -> impl Iterator<Item = &str>`

- [ ] **Step 1: Write the failing tests.**

```rust
#[test] fn empty_list_is_inert() { let mut c = ListCursor::default(); c.move_by(5); c.move_by(-5); c.select(3);
    assert_eq!(c.selected(), None); c.scroll_into_view(10); assert_eq!(c.offset(), 0); }
#[test] fn cursor_clamps_and_scrolls() { let mut c = ListCursor::default(); c.set_len(10); c.move_by(100);
    assert_eq!(c.selected(), Some(9)); c.scroll_into_view(4); assert_eq!(c.offset(), 6);
    c.set_len(3); assert_eq!(c.selected(), Some(2)); c.scroll_into_view(4); assert_eq!(c.offset(), 0); }
#[test] fn buffer_caps_and_follows() { let mut b = LogBuffer::new(3); b.extend((0..5).map(|i| i.to_string()));
    assert_eq!(b.len(), 3); assert_eq!(b.visible(2).collect::<Vec<_>>(), ["3", "4"]); }
#[test] fn not_following_keeps_view_stable_while_evicting() {
    let mut b = LogBuffer::new(5); b.extend((0..5).map(|i| i.to_string()));
    b.scroll_by(-2, 2);                                   // viewing "1","2"; follow turns off
    assert!(!b.follow()); assert_eq!(b.visible(2).collect::<Vec<_>>(), ["1", "2"]);
    b.push("5".into());                                   // evicts "0"
    assert_eq!(b.visible(2).collect::<Vec<_>>(), ["1", "2"]);
    b.extend((6..20).map(|i| i.to_string()));             // view content evicted entirely
    assert_eq!(b.visible(2).collect::<Vec<_>>(), ["15", "16"]); // pinned to oldest retained, never out of range
}
#[test] fn refollow_jumps_to_bottom() { let mut b = LogBuffer::new(10); b.extend((0..10).map(|i| i.to_string()));
    b.scroll_by(-5, 3); b.set_follow(true); assert_eq!(b.visible(3).collect::<Vec<_>>(), ["7", "8", "9"]); }
```

- [ ] **Step 2:** Run `cargo test state::`. Expected: compile FAIL.
- [ ] **Step 3: Implement both types.** `LogBuffer` stores a `VecDeque<String>` and an absolute `top` index. When following, `top = len - viewport`. On eviction while not following, `top` is decremented, saturating at 0.
- [ ] **Step 4:** Run the quality gate. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat: list cursor and bounded log buffer`.

---

### Task 4: App state and update (keyboard, selection, tabs, simulated streaming)

**Files:**
- Create: `src/app.rs`
- Test: unit tests in `src/app.rs`

**Interfaces:**
- Consumes: Tasks 1–3
- Produces:
  - `pub enum AppEvent { Key(KeyEvent), Mouse(MouseEvent), Resize(u16, u16), Tick }`, using the crossterm types from `ratatui::crossterm::event`
  - `pub enum Focus { List(ResourceKind), Viewer }`
  - `pub enum ViewerTab { Logs, Summary, Events, Yaml, Containers, Details, Workloads, Endpoints, MountedBy, Claim, StorageDetails }`, with `fn title(self) -> &'static str`. Titles in order: "Logs", "Summary", "Events", "YAML", "Containers", "Details", "Workloads", "Endpoints", "Mounted by", "Claim", "Storage details".
  - `pub fn tabs_for(r: &ResourceRef) -> &'static [ViewerTab]`, exactly as in design §4.4, first entry is the default:
    - Pod: Logs, Summary, Events, YAML, Containers
    - Image: Details, Workloads
    - Service: Details, Endpoints, Events, YAML
    - Pvc: Details, MountedBy, Events, YAML
    - Pv: Details, Claim, StorageDetails, YAML
  - `pub struct AppOptions { pub plain: bool, pub context: Option<String>, pub namespace: Option<String>, pub all_namespaces: bool }`
  - `pub struct App` with pub fields:
    - `snapshot: ClusterSnapshot`
    - `images: Vec<ImageUsage>`
    - `cursors: [ListCursor; 4]`, indexed by `ResourceKind::number() - 1`
    - `focus: Focus`
    - `selected: Option<ResourceRef>`
    - `tab: usize`
    - `container: usize`
    - `logs: LogBuffer`
    - `viewer_scroll: u16`
    - `zoom: bool`
    - `help: bool`
    - `quit: bool`
    - `plain: bool`
    - `hits: HitMap` (the type is created empty in this task; Task 6 fills it)
    - `size: (u16, u16)`
  - `App::new(snapshot: ClusterSnapshot, opts: AppOptions) -> App`: the CLI context and namespace override the snapshot's labels. The initial state matches PDF page 1: focus on Pods, cursor 0, `selected = Pod("api-7c9d8f-x2k4")`, Logs tab, logs filled from `FakeLogs::initial(.., 500)`.
  - `App::update(&mut self, ev: AppEvent)`
  - `App::row_ref(&self, kind: ResourceKind, idx: usize) -> Option<ResourceRef>`
  - `App::current_tabs(&self) -> &'static [ViewerTab]`
  - `App::current_containers(&self) -> Vec<SafeText>` lists non-init containers, then init containers
  - `App::select(&mut self, r: ResourceRef)`: resets `tab = 0`, `container = 0` and `viewer_scroll = 0`, and clears and refills logs when the ref is a pod

**Key map:**

| Key | Action |
|---|---|
| `q`, `Ctrl+C` | quit |
| `↑`/`k`, `↓`/`j` | move the cursor (List focus) or scroll the viewer (Viewer focus; on the Logs tab this means `logs.scroll_by`) |
| `PgUp`/`PgDn` | ±10 |
| `Enter` | select the row under the cursor (List focus) |
| `Tab` / `Shift+Tab` (`BackTab`) | cycle focus: Pods → Images → Services → Storage → Viewer → … |
| `1`–`4` | focus that list |
| `←`/`→` | previous/next viewer tab, wrapping |
| `c` | next container (Pod Logs only); refills logs |
| `f` | toggle follow |
| `z` | toggle zoom |
| `?` | toggle help |
| `Esc` | close help, else leave zoom |

While help is open, every key except `?`, `Esc` and `q` is swallowed. On `Tick`, if the selected pod is on its Logs tab, `next_line` is called once every 2 ticks and the result pushed (≈ the 437ms cadence at a 250ms tick, which is close enough).

- [ ] **Step 1: Write the failing tests.** Use helpers `key(KeyCode)` and `app()` (= `App::new(sample_snapshot(), AppOptions::default())`).

```rust
#[test] fn starts_like_pdf_page1() { let a = app(); assert_eq!(a.focus, Focus::List(ResourceKind::Pods));
    assert_eq!(a.selected, Some(ResourceRef::Pod("api-7c9d8f-x2k4".into()))); assert_eq!(a.current_tabs()[a.tab], ViewerTab::Logs);
    assert_eq!(a.logs.len(), 34); }
#[test] fn cursor_moves_but_selection_waits_for_enter() { let mut a = app(); a.update(key(KeyCode::Down));
    assert_eq!(a.selected, Some(ResourceRef::Pod("api-7c9d8f-x2k4".into())));
    a.update(key(KeyCode::Enter)); assert_eq!(a.selected, Some(ResourceRef::Pod("api-7c9d8f-q8n1".into()))); }
#[test] fn selecting_service_opens_details() { let mut a = app(); a.update(key(KeyCode::Char('3')));
    a.update(key(KeyCode::Down)); a.update(key(KeyCode::Down)); a.update(key(KeyCode::Enter));
    assert_eq!(a.selected, Some(ResourceRef::Service("billing".into()))); assert_eq!(a.current_tabs()[a.tab], ViewerTab::Details); }
#[test] fn tabs_wrap() { let mut a = app(); a.update(key(KeyCode::Left)); assert_eq!(a.current_tabs()[a.tab], ViewerTab::Containers); }
#[test] fn switching_pod_replaces_log_stream() { let mut a = app(); for _ in 0..8 { a.update(AppEvent::Tick); } assert!(a.logs.len() > 34);
    a.update(key(KeyCode::Down)); a.update(key(KeyCode::Enter)); assert_eq!(a.logs.len(), 34); }
#[test] fn container_cycle() { let mut a = app(); a.update(key(KeyCode::Char('c')));
    assert_eq!(a.current_containers()[a.container].as_str(), "envoy"); }
#[test] fn help_swallows_keys() { let mut a = app(); a.update(key(KeyCode::Char('?'))); a.update(key(KeyCode::Char('3')));
    assert_eq!(a.focus, Focus::List(ResourceKind::Pods)); a.update(key(KeyCode::Esc)); assert!(!a.help); }
#[test] fn ctrl_c_quits() { let mut a = app(); a.update(AppEvent::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))); assert!(a.quit); }
#[test] fn empty_services_enter_is_noop() { let mut s = sample_snapshot(); s.services.clear(); let mut a = App::new(s, Default::default());
    a.update(key(KeyCode::Char('3'))); a.update(key(KeyCode::Enter)); assert!(matches!(a.selected, Some(ResourceRef::Pod(_)))); }
```

- [ ] **Step 2:** Run `cargo test app::`. Expected: compile FAIL.
- [ ] **Step 3: Implement `App`.** List row order:
  - Pods: snapshot order.
  - Images: `images_in_use` order.
  - Services: snapshot order.
  - Storage: PVCs first, then PVs, each group in snapshot order.

  Ignore `KeyEventKind::Release` so Windows terminals don't double-fire.
- [ ] **Step 4:** Run the quality gate. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat: app state machine with keyboard navigation and simulated log streaming`.

---

### Task 5: Layout, header, lists, footer, help, theme

**Files:**
- Create: `src/ui/mod.rs`, `src/ui/theme.rs`, `src/ui/layout.rs`, `src/ui/header.rs`, `src/ui/lists.rs`, `src/ui/footer.rs`, `src/ui/help.rs`, `src/ui/hits.rs` (type only: `pub struct HitMap`, which derives `Default`; filled in Task 6), `src/ui/viewer/mod.rs` (placeholder that draws the bordered pane with title `Pod api-7c9d8f-x2k4` only)
- Test: `tests/support/mod.rs`, `tests/render_wide.rs`, `tests/render_narrow.rs`

**Interfaces:**
- Consumes: `App` (Task 4)
- Produces:
  - `pub fn draw(frame: &mut Frame, app: &mut App)`
  - `pub enum LayoutMode { Wide, Narrow, TooSmall }`
  - `pub fn layout_mode(area: Rect) -> LayoutMode`
  - `pub struct Regions { pub header: Rect, pub footer: Rect, pub context: Option<Rect>, pub list_tabs: Option<Rect>, pub lists: Vec<(ResourceKind, Rect)>, pub viewer: Rect }`
  - `pub fn regions(area: Rect, mode: LayoutMode, focused_list: ResourceKind, zoom: bool) -> Regions`
  - `pub struct Theme { … Style fields: ok, warn, err, muted, accent, cursor_focused, cursor_unfocused, border_focused, border }` with `Theme::color()` and `Theme::plain()`
  - Test helper: `pub fn render_to_string(app: &mut App, w: u16, h: u16) -> String`, which renders on ratatui `TestBackend` and joins buffer rows with `\n`

**Layout rules:**

Wide:
- header 1 row, footer 1 row
- left column `clamp(width * 2 / 5, 56, 80)` columns wide, holding:
  - a `[0]─Context` box 3 rows high
  - the four list boxes, splitting the rest equally with `Constraint::Fill(1)`
- viewer takes the right side
- zoom makes the viewer take everything between header and footer

Narrow:
- header 1 row
- list tab strip 3 rows: `[1] Pods 6  [2] Images 7  [3] Services 5  [4] Storage 5`, with the focused list highlighted
- the focused list: rows + 3, at most 40% of the height
- the viewer: the rest
- footer 1 row
- zoom works the same as in Wide

Content:
- Pane titles follow the PDF, e.g. `[1]─Pods 6` with `ns payments` on the right and `derived from pod specs` on the Images pane.
- Columns:
  - Pods: `NAME STATUS READY RST AGE`
  - Images: `REPOSITORY TAG / DIGEST USED`
  - Services: `NAME TYPE CLUSTER-IP PORTS EP`; narrow drops `CLUSTER-IP`
  - Storage: `KIND NAME STATUS CAP MODE CLASS`
- Colours:
  - status column: `ok` for Running/Bound, `warn` for Pending/Released, `err` for CrashLoopBackOff/Failed, `muted` for Completed
  - `EP` column: `warn` when it is 0
  - restart counts: `warn` when greater than 0, `err` when 10 or more
- The row the viewer is showing gets a `›` marker in a 1-column gutter. The cursor row is `cursor_focused` when the list has focus and `cursor_unfocused` otherwise. Marker and cursor are visually distinct.
- Every cell goes through `truncate(cell, column_width)`.

Header:
- Wide: `Kubeglass  ctx dev-east ▾  ns payments ▾  ● Connected  api v1.31.4 · eks  ⟳ sample data · every 10s`, with `READ-ONLY` right-aligned in a bordered-looking span.
- Narrow: `KG ctx dev-east ns payments ● Connected`, with `RO` right-aligned.

Footer:
- Wide: `↑↓ navigate  enter select  tab focus  1-4 jump to list  ←→ tabs  z zoom  ? help  q quit`, with `v0.1.0 · ? help` on the right. The version comes from `env!("CARGO_PKG_VERSION")`.
- Narrow: a shortened version of the same.

Other:
- The help overlay is a centred box listing the Task 4 key map.
- TooSmall renders only `Terminal too small — need at least 60×16`.

- [ ] **Step 1: Write the failing tests.**

```rust
// tests/render_wide.rs
#[test] fn wide_matches_pdf_page1_text() { let mut a = app(); let s = render_to_string(&mut a, 160, 48);
    for needle in ["Kubeglass", "ctx dev-east", "ns payments", "READ-ONLY", "[1]─Pods 6", "[2]─Images 7", "[3]─Services 5",
                   "[4]─Storage 5", "api-7c9d8f-x2k4", "CrashLoopBackOff", "@sha256:9f2c…e81a", "80→8080/TCP", "443:31443/TCP",
                   "pvc-77b2d4f1-141f", "Released", "1-4 jump to list"] { assert!(s.contains(needle), "missing {needle}\n{s}"); } }
#[test] fn status_has_symbol_and_label() { let s = render_to_string(&mut app(), 160, 48);
    assert!(s.contains("✕ billing-0") || s.contains("✕") && s.contains("CrashLoopBackOff")); assert!(s.contains("◌")); assert!(s.contains("✓")); }
#[test] fn too_small_never_panics() { for (w, h) in [(1, 1), (40, 10), (59, 30), (200, 15), (0, 0)] {
    let s = render_to_string(&mut app(), w, h); if w >= 40 && h >= 1 { assert!(s.contains("Terminal too small")); } } }
#[test] fn hostile_names_are_clean_and_truncated() { let mut snap = sample_snapshot();
    snap.pods[0].name = SafeText::from("\x1b]0;pwn\x07evil-\x1b[31m日本語ポッド-with-a-very-very-long-name-indeed-0123456789");
    let mut a = App::new(snap, Default::default()); let s = render_to_string(&mut a, 160, 48);
    assert!(!s.contains('\x1b')); assert!(s.contains("evil-日本語")); assert!(s.contains('…')); }
#[test] fn empty_list_shows_placeholder() { let mut snap = sample_snapshot(); snap.services.clear();
    let s = render_to_string(&mut App::new(snap, Default::default()), 160, 48); assert!(s.contains("No services in payments")); }
#[test] fn plain_theme_has_no_colors() { let mut a = App::new(sample_snapshot(), AppOptions { plain: true, ..Default::default() });
    let buf = render_to_buffer(&mut a, 160, 48);
    assert!(buf.content().iter().all(|c| c.fg == Color::Reset && c.bg == Color::Reset)); }
// tests/render_narrow.rs
#[test] fn narrow_matches_pdf_page2_text() { let mut a = app(); a.update(key(KeyCode::Char('3'))); let s = render_to_string(&mut a, 92, 48);
    for needle in ["KG", "RO", "[1] Pods 6", "[3] Services 5", "payments-lb"] { assert!(s.contains(needle), "missing {needle}"); }
    assert!(!s.contains("api-7c9d8f-q8n1"), "only the focused list is shown in narrow mode"); }
#[test] fn layout_modes() { assert_eq!(layout_mode(Rect::new(0,0,160,48)), LayoutMode::Wide);
    assert_eq!(layout_mode(Rect::new(0,0,92,48)), LayoutMode::Narrow); assert_eq!(layout_mode(Rect::new(0,0,59,48)), LayoutMode::TooSmall); }
```

(`render_to_buffer` is the same helper as `render_to_string`, but it returns the `Buffer`. Put both in `tests/support/mod.rs`.)

- [ ] **Step 2:** Run `cargo test --test render_wide --test render_narrow`. Expected: FAIL.
- [ ] **Step 3: Implement theme, layout, header, lists, footer and help.** Build the tables with `ratatui::widgets::Table` and use `Block::bordered()` titles for the panes. `draw` begins by calling `layout_mode(frame.area())`. On TooSmall it renders the message and returns. Use `Paragraph` with `wrap` so a 1-column terminal doesn't panic.
- [ ] **Step 4:** Run the quality gate. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat: wide and narrow layouts with lists, header, footer, help`.

---

### Task 6: Viewer content per resource + mouse hit-testing

**Files:**
- Create: `src/ui/viewer/pod.rs`, `src/ui/viewer/image.rs`, `src/ui/viewer/service.rs`, `src/ui/viewer/storage.rs`
- Modify: `src/ui/viewer/mod.rs`, `src/ui/hits.rs`, `src/ui/lists.rs` (record row and pane rects), `src/app.rs` (handle `AppEvent::Mouse` and `AppEvent::Resize`)
- Test: additions to `tests/render_wide.rs` and `tests/render_narrow.rs`; new `tests/mouse.rs`

**Interfaces:**
- Consumes: `ViewerTab`, `tabs_for`, `App`, `Theme`, `derive::pods_matching`, `fake::sample_yaml`, `snapshot.events`
- Produces:
  - `pub enum Hit { ListRow(ResourceKind, usize), ListPane(ResourceKind), ListTab(ResourceKind), ViewerTab(usize), Viewer }`
  - `HitMap { rows: Vec<(Rect, ResourceKind, usize)>, panes: Vec<(Rect, ResourceKind)>, list_tabs: Vec<(Rect, ResourceKind)>, tabs: Vec<(Rect, usize)>, viewer: Rect }` with:
    - `fn clear(&mut self)`
    - `fn hit(&self, x: u16, y: u16) -> Option<Hit>`: checks the most specific region first
  - `pub fn viewer_lines(app: &App, tab: ViewerTab, theme: &Theme, width: u16) -> Vec<Line<'static>>` (non-Logs tabs)

**Viewer content** (match the PDF wording):

- **Viewer chrome:**
  - Tabs are drawn as `Logs - Summary - Events - YAML - Containers`, with the active tab underlined in `accent`.
  - The title is right-aligned: `Pod api-7c9d8f-x2k4`.
  - A bottom hint line, e.g. `←→ tabs` (and `c container · f follow` on Logs).
- **Pod Logs:**
  - Toolbar: `container ▸ api  envoy   follow ● on   tail 500   previous n/a`. The active container is accented.
  - Body: `logs.visible(rows)`, with the level column coloured (INFO ok, WARN warn, ERROR err, DEBUG muted).
  - Status bar: `● streaming (sample) · {n} lines buffered (cap 5000)`.
- **Pending pod** (D6): instead of logs, show `◌ Container ledger-sync has not started (ContainerCreating)` followed by that pod's events.
- **Pod Summary:** name, namespace, status, node, IP, owner, ready, restarts, age, labels.
- **Pod Containers:** one block per container with name, image, ready, restarts, and the `init` tag.
- **Events tab:** a table `TYPE REASON AGE COUNT MESSAGE` filtered to the selected object, or `No events for this object` when there are none.
- **YAML tab:** `sample_yaml`. Scrolls with `viewer_scroll`.
- **Image Details:** repository, tag/digest, and the full raw reference with no truncation; it wraps instead. Then the line `Registry metadata: not retrieved (no registry calls in this version)`.
- **Image Workloads:** a `pod → container` list.
- **Service Details:** sections `Service`, `Endpoints` and `Matching pods`, matching PDF page 2.
  - When there are zero ready endpoints: `⚠ No ready endpoints. Traffic to this Service will fail.` and `{m} pod matches the selector; 0 are Ready.`, using "pods match" when the count isn't 1.
  - Matching pods are drawn as `→ billing-0  CrashLoopBackOff  [derived]`.
  - The section ends with `Selector matches show configuration, not network reachability.`
- **Service Endpoints:** `{n} ready endpoints`, then the matching-pod IPs labelled `[derived]`.
- **PVC Details / Mounted by:**
  - Details: status, capacity, access, class and bound PV.
  - Mounted by: the `mounted_by` pods labelled `[derived]`.
  - Both end with `Kubernetes metadata only — storage contents are not inspected.`
- **PV Details / Claim / Storage details:** the same fields as PVC Details, plus the claim reference. Released PVs show `Released — claim was deleted; volume not reusable until reclaimed.`
- Nothing selected: `Nothing selected`.

**Mouse behaviour:**

| Mouse action | Effect |
|---|---|
| Left click on a list row | focus the list, set the cursor, and select (same as Enter) |
| Click a list pane's empty area | focus it |
| Click a narrow list tab | focus that list |
| Click a viewer tab | switch to that tab |
| Click the viewer | focus the viewer |
| Wheel over a list | move its cursor by ±3 |
| Wheel over the viewer | scroll it by ±3 |

On `Resize(w, h)`, set `size`. When the new layout is Narrow, the focused list stays focused and becomes the visible tab. Zoom is unchanged.

- [ ] **Step 1: Write the failing tests.**

```rust
// tests/render_narrow.rs
#[test] fn service_billing_matches_pdf_page2() { let mut a = app(); for k in ['3'] { a.update(key(KeyCode::Char(k))); }
    a.update(key(KeyCode::Down)); a.update(key(KeyCode::Down)); a.update(key(KeyCode::Enter));
    let s = render_to_string(&mut a, 92, 48);
    for n in ["Details - Endpoints - Events - YAML", "Service billing", "app=billing", "No ready endpoints.",
              "1 pod matches the selector; 0 are Ready.", "billing-0", "[derived]", "not network reachability"] {
        assert!(s.contains(n), "missing {n}\n{s}"); } }
// tests/render_wide.rs
#[test] fn pod_logs_toolbar_and_status() { let s = render_to_string(&mut app(), 160, 48);
    for n in ["Logs - Summary - Events - YAML - Containers", "Pod api-7c9d8f-x2k4", "container ▸ api", "envoy",
              "follow ● on", "tail 500", "34 lines buffered (cap 5000)", "12:04:11.000 INFO"] { assert!(s.contains(n), "missing {n}"); } }
#[test] fn pending_pod_explains_instead_of_empty_logs() { let mut a = app(); for _ in 0..5 { a.update(key(KeyCode::Down)); }
    a.update(key(KeyCode::Enter)); let s = render_to_string(&mut a, 160, 48); assert!(s.contains("has not started (ContainerCreating)")); }
#[test] fn image_details_label_registry_unavailable() { let mut a = app(); a.update(key(KeyCode::Char('2'))); a.update(key(KeyCode::Enter));
    assert!(render_to_string(&mut a, 160, 48).contains("Registry metadata: not retrieved")); }
// tests/mouse.rs
#[test] fn click_row_selects_it() { let mut a = app(); render_to_string(&mut a, 160, 48);
    let (rect, _, _) = *a.hits.rows.iter().find(|(_, k, i)| *k == ResourceKind::Services && *i == 2).unwrap();
    a.update(click(rect.x + 1, rect.y)); assert_eq!(a.selected, Some(ResourceRef::Service("billing".into())));
    assert_eq!(a.focus, Focus::List(ResourceKind::Services)); }
#[test] fn click_viewer_tab_switches() { let mut a = app(); render_to_string(&mut a, 160, 48);
    let (rect, i) = *a.hits.tabs.iter().find(|(_, i)| *i == 3).unwrap(); a.update(click(rect.x, rect.y));
    assert_eq!(a.current_tabs()[a.tab], ViewerTab::Yaml); let _ = i; }
#[test] fn resize_to_narrow_keeps_focus_and_zoom() { let mut a = app(); a.update(key(KeyCode::Char('4'))); a.update(key(KeyCode::Char('z')));
    render_to_string(&mut a, 160, 48); a.update(AppEvent::Resize(92, 48)); let s = render_to_string(&mut a, 92, 48);
    assert_eq!(a.focus, Focus::List(ResourceKind::Storage)); assert!(a.zoom); assert!(s.contains("Pod api-7c9d8f-x2k4"));
    a.update(key(KeyCode::Char('z'))); assert!(render_to_string(&mut a, 92, 48).contains("[4] Storage 5")); }
```

(`click(x, y)` builds `AppEvent::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: x, row: y, modifiers: KeyModifiers::NONE })`. Put it in `tests/support`.)

- [ ] **Step 2:** Run `cargo test --tests`. Expected: the new tests FAIL.
- [ ] **Step 3: Implement the viewer modules.** Clear `app.hits` at the start of `draw`. Each pane pushes its rects. Tab rects are computed from the cumulative display width of the tab titles.
- [ ] **Step 4: Implement mouse handling and Resize in `App::update`.** Ignore events that hit nothing.
- [ ] **Step 5:** Run the quality gate. Expected: PASS.
- [ ] **Step 6: Commit** with message `feat: per-resource viewer content and mouse hit-testing`.

---

### Task 7: Binary — CLI, terminal lifecycle, event pump, docs

**Files:**
- Create: `src/cli.rs`, `docs/keybindings.md` (the Task 4 key map plus the mouse table)
- Modify: `src/main.rs`, `README.md` (install from source, run, keys, "sample data only" note)
- Test: unit tests in `src/cli.rs`

**Interfaces:**
- Consumes: `App`, `AppOptions`, `AppEvent`, `ui::draw`, `fake::sample_snapshot`
- Produces: `pub struct Args { pub kubeconfig: Option<PathBuf>, pub context: Option<String>, pub namespace: Option<String>, pub all_namespaces: bool, pub no_color: bool }` with `fn to_options(&self, no_color_env: Option<&str>) -> AppOptions`. `NO_COLOR` set and non-empty means plain.

- [ ] **Step 1: Write the failing tests.**

```rust
#[test] fn kubectl_style_flags() { let a = Args::try_parse_from(["kglass", "-n", "payments", "--context", "dev-east"]).unwrap();
    assert_eq!(a.namespace.as_deref(), Some("payments")); assert_eq!(a.context.as_deref(), Some("dev-east"));
    assert!(Args::try_parse_from(["kglass", "-A"]).unwrap().all_namespaces); }
#[test] fn no_color_env() { let a = Args::try_parse_from(["kglass"]).unwrap();
    assert!(a.to_options(Some("1")).plain); assert!(!a.to_options(Some("")).plain); assert!(!a.to_options(None).plain); }
```

- [ ] **Step 2:** Run `cargo test cli::`. Expected: compile FAIL.
- [ ] **Step 3: Implement `Args`** with clap derive. Set `#[command(name = "kglass", version, about = "Read-only Kubernetes cluster viewer")]`.
- [ ] **Step 4: Implement `main`.**
  1. Install `color_eyre`.
  2. Parse args.
  3. Call `ratatui::init()`, then `execute!(stdout(), EnableMouseCapture)`.
  4. Wrap the panic hook: take the current hook and install one that runs `DisableMouseCapture` and then calls the previous hook. `ratatui::init` already restores raw mode and the alternate screen.
  5. Loop:
     - `terminal.draw(|f| ui::draw(f, &mut app))`
     - poll for up to the time left in the 250ms tick, mapping crossterm `Event::{Key, Mouse, Resize}` to `AppEvent`
     - send `Tick` when the tick elapses
     - stop on `app.quit`
  6. After the loop, run `DisableMouseCapture` and `ratatui::restore()`, also on the error path.
- [ ] **Step 5: Manual acceptance.** Run `cargo run` in iTerm2 or Terminal.app and check that:
  - The wide layout matches PDF page 1.
  - Shrinking the window to under 120 columns switches to narrow (PDF page 2). Under 60 columns shows the too-small message.
  - Click rows and tabs, and scroll with the wheel.
  - Every action also works with the keyboard alone.
  - `NO_COLOR=1 cargo run` shows no colours.
  - `q` and `Ctrl+C` both leave a clean prompt, and the mouse doesn't print escape codes afterwards.
  - Temporarily add `panic!()` on the `p` key, press it, and confirm the terminal is restored. Then remove it.
- [ ] **Step 6:** Run the quality gate plus `cargo build --release`. Expected: PASS.
- [ ] **Step 7: Commit** with message `feat: kglass binary with terminal lifecycle, CLI flags, keybinding docs`.
- [ ] **Step 8: Update `docs/design.md` §9** with the line "Phase 1 done <date>", plus any deviations. Commit with message `docs: mark phase 1 complete`.

---

## Phase 1 exit check (from design §6)
Matches PDF pages 1–2 on sample data, resizes cleanly, and exits cleanly even after a panic. Tasks 5–7 cover this, and Task 7 Step 5 checks it by hand.

## Deferred to later phases (on purpose, not forgotten)
- **Phase 2:**
  - filter (`/`)
  - refresh (`r`)
  - context and namespace pickers (`x`/`n`)
  - real kubeconfig loading (D15)
  - loading, 403 and stale states
  - selection kept across refresh
  - tokio and the event channel
- **Phase 3:**
  - log search
  - wrap and timestamp toggles
  - previous-container logs
  - Enter-to-follow-relationship navigation
  - ghost rows
  - auth error screen
- **Phase 4:** CI workflow, release binaries, `docs/rbac.md`.
