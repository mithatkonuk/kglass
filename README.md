# Kubeglass (`kglass`)

A fast, **read-only** terminal viewer for Kubernetes clusters. Pick a pod, image, service or volume from the lists on the left, and the large viewer on the right shows its logs, details and related resources.

> **Status: phase 1 prototype.** It runs on built-in sample data only and does not connect to a cluster yet. Cluster access (kubeconfig, contexts, namespaces) arrives in phase 2.

## Features (prototype)

- Wide layout with all four lists (Pods, Images, Services, Storage) next to a large viewer. Below 120 columns it switches to a narrow layout with list tabs.
- Pods open on their **logs**: a container switcher, follow mode, a buffer capped at 5000 lines, and simulated streaming.
- **Summary, Events, YAML and Containers** tabs for pods. Service details include endpoint health and selector-matched pods. Claim/volume details show storage relationships.
- Inferred relationships are labelled `[derived]`, because a selector match shows configuration, not network reachability.
- Full keyboard control plus optional mouse. See [docs/keybindings.md](docs/keybindings.md).
- Status is never shown by colour alone. `--no-color` or `NO_COLOR=1` turns colours off.
- Cluster text is sanitized before display, so escape sequences in names or logs can't control your terminal.

## Install

Requires Rust 1.88 or newer.

```bash
cargo install --locked --path .
```

This puts `kglass` in `~/.cargo/bin`. `--locked` builds with the dependency versions tested in `Cargo.lock`.

## Run from source

```bash
cargo run --release
```

The flags match kubectl. In phase 1 they only change the labels shown in the header:

```bash
cargo run -- --context dev-east -n payments
```

## Development

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

The design and roadmap are in [docs/design.md](docs/design.md), and the phase 1 plan is in [docs/plan-phase1.md](docs/plan-phase1.md).

## License

Licensed under either the [Apache License 2.0](LICENSE-APACHE) or the [MIT license](LICENSE-MIT), at your option.
