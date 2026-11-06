# Kubeglass Phase 4 — Hardening & Public Beta Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Kubeglass ready to publish:
- all-namespaces support
- a verified RBAC guide
- CI and release automation
- open-source project documents
- a README screenshot

Publishing the repository itself needs the owner's explicit go-ahead.

**Spec:** [`design.md`](design.md) §6, phase 4. All standing rules from [`plan-phase2.md`](plan-phase2.md) still apply.

## Decisions (my calls)

- **`-A` keys namespaced objects as `namespace/name`.** Kubernetes names cannot contain `/`, so the key is unambiguous. This keeps every viewer, relation and test path unchanged in single-namespace mode. The executor splits the key back into namespace and name for API calls. Picking a namespace with `n` leaves all-namespaces mode.
- **Relations must stay within one namespace.** That means a selector match, an EndpointSlice match and a PVC mount, in both modes. This also fixes a latent all-namespaces bug before it can happen.
- **The RBAC guide is generated from the same constant the code uses** (`fetch::PERMISSIONS`), and a test keeps them in sync.
- **Release binaries are built by GitHub Actions on tags** (macOS arm64/x86_64, Linux x86_64/arm64) and published with SHA-256 checksums. Workflows are written and syntax-checked locally. They only run once a remote exists.
- **The README screenshot is an SVG rendered from `--demo` data** by an example program, so it is reproducible and contains no cluster data.

## Tasks

### T1: `-A/--all-namespaces`
- `fetch::Scope { Namespace(String), All }`. `fetch_lists(client, &Scope, now)` uses `Api::all` for namespaced kinds when the scope is `All`.
- **Qualification:** in `All` mode, names become `ns/name` for pods, services and PVCs, for mount lists and for event objects.
- **Same-namespace rules:** `convert::service` matches slices from the same namespace only; `convert::pvc` counts mounts from same-namespace pods only; `derive::pods_matching` takes the service namespace.
- **Executor:** holds a `Scope`. `split_key("ns/name")` gives the namespace and name for logs and YAML.
- **App:** `LiveTarget.all_namespaces`; the header shows `ns all`; a PV's claim relation uses the full `ns/name` key; the `n` picker switches to single-namespace mode.
- **Tests:**
  - convert: slices and mounts are scoped to their namespace
  - derive: matching pods are limited to the service's namespace
  - fetch: All scope lists from `/api/v1/pods` and qualifies names
  - executor: split keys for logs and YAML
  - app: header label, PV → PVC relation, the namespace pick leaves `-A`
  - cli: `-A` accepted

### T2: RBAC guide
- `fetch::PERMISSIONS: &[(&str /*apiGroup*/, &str /*resource*/, &[&str] /*verbs*/, Scope)]` lists every call kglass makes.
- `docs/rbac.md` includes a namespace `Role`, an optional `ClusterRole` (PVs, namespaces), and the bindings.
- Test `rbac_doc_matches_code` parses the YAML blocks in `docs/rbac.md` and checks that they grant exactly `PERMISSIONS`: nothing missing, nothing extra, no write verbs, no Secrets.

### T3: CI & releases
- `.github/workflows/ci.yml`: fmt, clippy, test on ubuntu-latest and macos-latest with stable, plus a build at MSRV 1.89.
- `.github/workflows/release.yml`: on a `v*` tag, build `--locked --release` for 4 targets, package `kglass-<ver>-<target>.tar.gz` with `sha256` files, and attach them to a GitHub release.
- **Local checks:** parse both workflows as YAML; run `cargo check --target x86_64-unknown-linux-gnu`.

### T4: Open-source documents & screenshot
- `SECURITY.md` (private reporting; no credentials in issues), `CONTRIBUTING.md` (the gate commands, TDD, the read-only rule), and `CODE_OF_CONDUCT.md` (short, linking the Contributor Covenant).
- Cargo metadata (`repository` is added once the repo exists).
- `examples/screenshot.rs` renders `--demo` at 160×48 to `docs/screenshot.svg`, with colours. The README embeds it.

### T5: Acceptance, review, docs
- Real cluster: `kglass -A` (read-only) shows multiple namespaces and opens logs for a pod outside the default namespace.
- Independent review; fix its findings with tests; update the design note and memory; reinstall.

### Publishing (owner's decision)
Create the GitHub repository (name, owner, visibility), add `repository` to Cargo.toml, and push. **Not done without an explicit yes.**
