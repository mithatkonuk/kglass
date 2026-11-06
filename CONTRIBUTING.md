# Contributing to Kubeglass

Thanks for helping! A few rules keep kglass safe and pleasant to work on.

## Ground rules

- **Read-only.** Release 1 never changes cluster state. Write actions (delete, restart, scale) need a separate, reviewed safety design first, so please open an issue before writing any.
- **Cluster text is untrusted.** Wrap every string that comes from the cluster in `SafeText` at the boundary (`src/cluster/convert.rs`, log ingest). Never build UI spans from raw cluster strings.
- **No credentials anywhere.** No `Debug` derive over kubeconfig types, and no plugin output in errors.
- **Pure core.** `App::update` performs no I/O. Side effects are `Command`s, run by `src/runtime.rs`.

## Workflow

1. Write a failing test first, then make it pass.
2. Before every commit, run the same gate as CI:

   ```bash
   cargo fmt --check
   cargo clippy --all-targets -- -D warnings
   cargo test
   ```

3. Keep pull requests small and focused, with a clear description. Update `docs/` and the README when behaviour changes.

Useful tools:

- `cargo run -- --demo` runs the UI on sample data.
- `cargo run --example probe` runs a read-only check of your current kubeconfig.
- `cargo run --example screenshot` regenerates `docs/screenshot.svg`.

The design and roadmap are in [docs/design.md](docs/design.md).

## Conduct

Please follow the [Code of Conduct](CODE_OF_CONDUCT.md).
