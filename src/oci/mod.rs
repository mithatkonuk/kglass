//! Local containers through a Docker-compatible CLI (`docker`, `podman`, `nerdctl`).
//! Read-only: `ps`, `container inspect`, `volume inspect`, `logs` and `version` only.

pub mod convert;
pub mod engine;

pub use engine::Engine;

#[cfg(test)]
pub(crate) mod testing;
