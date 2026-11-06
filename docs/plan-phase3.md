# Kubeglass Phase 3 — Viewer Polish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the live viewer pleasant for daily debugging:
- readable structured logs
- log search
- previous-container logs for crash loops
- jumping from a resource to the resources it relates to
- honest notes when a list is cut off or credentials fail

**Architecture:** All phase 2 rules still apply: a pure `App::update`, a `Command` queue, and `SafeText` at the boundaries.
- Log formatting is a pure function applied at render time. Stored lines stay raw, so search and the raw view see the original text.
- Related resources come from one pure function, `App::related()`, so the viewer, the keys and the tests all agree on the same list.

**Spec:** [`design.md`](design.md) §4.3–§4.5 and the phase 3 backlog in §9. Phase 2 rules: [`plan-phase2.md`](plan-phase2.md) Global Constraints.

## Scope decisions (my calls)

- **`-A/--all-namespaces` moves to phase 4.** It needs `ResourceRef` to carry a namespace, a refactor of every resource path. The owner's workflow uses `kubens`, so it has the lowest value per risk.
- **Ghost rows are dropped.** The phase 2 "no longer present" banner plus the last known data already cover the use case.
- **JSON logs are shown formatted by default.** `J` toggles the raw line.

## Global Constraints

- **Read-only.** Previous-container logs use the same `get pods/log` permission, with `previous=true`.
- **Keys:**
  - `/` with the viewer focused on Logs searches the logs; with a list focused it still filters the list.
  - `p` toggles previous-container logs.
  - `J` toggles raw/formatted JSON.
  - With a related-resources section showing, `↑↓` move between related resources and `Enter` jumps to the one selected.
- Gate per task: fmt, clippy `-D warnings`, test. Commits end with the Co-Authored-By line.

## Review Focus

1. **Lines that are not JSON, invalid JSON, a JSON array, or a JSON number timestamp** are shown raw and never panic. Tested in T1.
2. **Searching while lines stream and the buffer evicts** keeps the view stable and never goes out of range. Tested in T2.
3. **`p` on a container with no previous instance** (restarts = 0): the API answers 400. kglass shows `no previous container (0 restarts)` without a request, and a 400 is shown as an ended stream. Tested in T3.
4. **Jumping to a related pod that is filtered out of the Pods list** clears that filter so the row is visible. Tested in T4.
5. **An Unauthorized error on every pane** shows the auth help once, in the viewer, not as four repeated errors. Tested in T5.

---

### T1: Readable structured log lines
- **Files:** `src/logfmt.rs` (new), `src/ui/viewer/pod.rs`, `src/app.rs` (`pub raw_logs: bool`, `J` key).
- **Interface:** `pub fn format_line(raw: &str) -> Option<String>` returns `{YYYY-MM-DD HH:MM:SS.mmm} {LEVEL:<5} {logger_short:<16} {message}` and appends ` ✕ {error.type}: {error.message}` when an error is present. Non-JSON input returns `None`, and the raw line is shown.
- **Key aliases** (dotted or nested):
  - timestamp: `@timestamp|timestamp|time|ts` (strings only)
  - level: `log.level|level|severity|lvl`
  - logger: `log.logger|logger|logger_name|caller`, shortened to the last `.` segment
  - message: `message|msg`
- **Tests:**
  - `ecs_line_formatted`: the real shape seen on the dev cluster
  - `nested_and_dotted_keys`
  - `error_fields_appended`
  - `plain_text_is_none`
  - `json_without_message_is_none`
  - `numeric_timestamp_falls_back`
  - `level_uppercased`
  - render: `json_logs_render_formatted_and_j_shows_raw`

### T2: Log search
- **Files:** `src/app.rs` (`log_search: Option<String>`, `log_search_editing: bool`), `src/state/log_buffer.rs` (`matching(&self, query) -> Vec<usize>`), `src/ui/viewer/pod.rs`.
- **Behaviour:**
  - `/` with the viewer focused on Logs starts the search, and typed text is captured.
  - `Enter` keeps the search and `Esc` clears it.
  - While a search is active, the body shows only matching lines (newest at the bottom when following) and the status reads `search "x" · N matches`.
  - Matching is a case-insensitive substring over the raw line.
- **Tests:**
  - `search_shows_only_matching_lines`
  - `search_counts_matches`
  - `esc_clears_search`
  - `search_survives_eviction`
  - `slash_in_list_still_filters_list`

### T3: Previous-container logs
- **Files:** `src/command.rs` (`StartLogs { previous: bool }`), `src/cluster/fetch.rs` (`log_lines(.., previous)`, `follow = !previous`), `src/app.rs` (`p` key, `logs_previous: bool`), `src/ui/viewer/pod.rs` (toolbar `previous on/off/n/a`).
- **Behaviour:**
  - The setting resets on every new selection.
  - `p` with 0 restarts makes no request and shows the notice.
  - With previous on, the stream ends normally: no auto-resume, and the state reads `previous logs (complete)`.
- **Tests:**
  - `p_requests_previous_logs`
  - `p_with_zero_restarts_shows_notice`
  - `previous_stream_is_not_resumed`
  - `toolbar_shows_previous_state`
  - fetch: `previous_param_sent`

### T4: Jump to related resources
- **Files:** `src/app.rs` (`pub fn related(&self) -> Vec<ResourceRef>`, `related_cursor`, `jump_to`), viewers for service, storage and image (highlight the cursor line and show the hint `enter jump`).
- **Relations:**
  - Service (Details/Endpoints) → its matching pods
  - PVC → the pods mounting it, then its bound PV
  - PV → its claim (PVC, by name)
  - Image (Workloads) → the pods using it
  - Pod Summary → its claimed PVCs (from mounts) and the services selecting it
- **Jumping:** `jump_to(r)` focuses the right list, clears that list's filter if the row is hidden, moves the cursor and selects. In the viewer, `↑↓` move the related cursor when there are related items; otherwise they scroll.
- **Tests:**
  - `service_related_pods`
  - `enter_jumps_to_pod_and_focuses_list`
  - `jump_clears_hiding_filter`
  - `pvc_related_pods_and_pv`
  - `pod_related_services_and_claims`
  - `no_related_keeps_scrolling`
  - render: `related_cursor_highlighted`

### T5: Truncation note, PVC Terminating, auth help
- **Truncation:** `fetch::Lists.truncated: Vec<ResourceKind>`. The `list` paging records when it stops at `MAX_ITEMS`. A truncated pane's title gets ` · first 5000`, and the disappearance check is skipped for that kind.
- **PVC Terminating:** a PVC with a deletion timestamp shows the status `Terminating`.
- **Auth help:** when the connection is Disconnected with an Unauthorized error and there is no data, the viewer shows:
  - `✕ Cannot authenticate to context <ctx>`
  - the reason
  - `Rancher: download a fresh kubeconfig (or token) from the Rancher UI, then press r.` (shown when the hint is rancher)
  - `Or press x to pick another context, q to quit.`
- **Tests:**
  - `paging_stops_at_cap_and_reports_truncation`: mock with a `continue` token
  - `truncated_title_and_no_false_disappearance`
  - `pvc_terminating`
  - `auth_help_shown_once`

### T6: Acceptance, review, docs
Run against the dev cluster through the probe tool and a pty. Get an independent review and fix what it finds, each fix with a test. Update the README, keybindings, the design note and memory, then reinstall.
