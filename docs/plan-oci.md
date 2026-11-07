# Kubeglass — Local Containers (`--oci`) Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `kglass --oci [COMMAND]` shows the local containers in the same screen, with the same keys and the same log features as Kubernetes pods. It is read-only.

**Decisions (owner):**
- It is engine-neutral, not Docker-only.
- It simply runs the container CLI: `docker` by default, or `podman` / `nerdctl` when named.

**Architecture:**
- **Engine layer:** `src/oci/` runs the CLI without a shell, through a `Runner` trait so tests can use a fake. It uses only these read-only commands:
  - `ps -aq --no-trunc`
  - `container inspect -- <ids>`
  - `logs --follow --tail 500 -- <id>`
  - `volume inspect -- <name>`
  - `version --format {{.Server.Version}}`
- **Conversion:** the JSON from `container inspect` (the shape Docker, Podman and nerdctl share) is converted into the existing model. The four panes, the viewer, logs, search, JSON formatting, filtering and jump-to-related then work unchanged.
- **Routing:** the executor sends commands to the OCI engine when one is set. The `App` core is unchanged except for its labels.

## Mapping (my calls)
| kglass pane | Local containers |
|---|---|
| `[1]` Pods | **Containers.** Status: `running` → Running, `exited 0` → Completed, `exited ≠0` → Failed, `restarting` → **Restarting** (new status), `created`/`paused` → Pending. Namespace = compose project. Restarts = `RestartCount`. |
| `[2]` Images | Derived from the containers, as now. |
| `[3]` Services | **Published ports**: one row per container that publishes ports (type `Published`, e.g. `18080→80/tcp`), linked to its container. |
| `[4]` Storage | **Volumes** (new kind `VOL`), with the containers mounting them. Bind mounts are not listed, since host paths are noisy and may be private. |
| `-n NAME` / `n` picker | Filter by compose project; the default is all containers. |
| Header | `ctx oci: docker  ns all  ● Connected  api docker 29.9.2`. |

**Errors:**
- If the command is missing, the header shows ``● Disconnected · `docker` command not found``.
- If the engine is down (e.g. "Cannot connect to the Docker daemon"), its first stderr line is shown, sanitized.

## Tasks (TDD, one commit each, all with `[skip ci]`)
1. **Model:** `PodStatus::Restarting`, `StorageKind::Volume` (treated like a claim), `ServiceType::Published`.
2. **`oci::convert`:** inspect JSON → `Pod`/`Service`/`StorageItem`. Fixtures cover running, exited 0/1, restarting, created, volume shared by two containers, and a bind mount ignored.
3. **`oci::engine`:** a `Runner` trait plus a process runner (tokio, `kill_on_drop`, 15s timeout). Provides `fetch_lists`, `version`, `projects`, `inspect_yaml`, and `log_lines` (stdout and stderr merged, sanitized, 8 KiB cap). Tested with a fake runner.
4. **Wiring:** CLI `--oci [COMMAND]` (conflicts with `--demo`, `--context`, `--kubeconfig`); the executor routes Refresh/StartLogs/FetchYaml/FetchNamespaces/SwitchTarget to the engine; `main` builds it.
5. **Acceptance:** a real Docker Desktop run with a throwaway container (`kglass-smoke`), an independent review, docs (README, keybindings, design note), and a reinstall.
