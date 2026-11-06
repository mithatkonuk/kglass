//! Side effects requested by the pure core. `App::update` queues these; the
//! runtime shell executes them and answers with `AppEvent::Data`.

use crate::model::ResourceRef;
use crate::text::SafeText;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Fetch all lists and the server version for the current target.
    Refresh { generation: u64 },
    /// Replace any running log stream with this container's (tail 500, follow).
    StartLogs {
        session: u64,
        pod: SafeText,
        container: SafeText,
        /// Logs of the previous (terminated) instance instead of following the current one.
        previous: bool,
    },
    /// Stop the running log stream, if any.
    StopLogs,
    /// List namespaces for the namespace picker.
    FetchNamespaces { generation: u64 },
    /// Use another context and/or namespace for this session (never written to kubeconfig).
    SwitchTarget {
        context: SafeText,
        namespace: SafeText,
    },
    /// Fetch the YAML of `target`.
    FetchYaml {
        generation: u64,
        target: ResourceRef,
    },
}
