//! Command-line flags. Names match kubectl so muscle memory carries over.

use std::path::PathBuf;

use clap::Parser;

use crate::app::AppOptions;

#[derive(Debug, Parser)]
#[command(
    name = "kglass",
    version,
    about = "Kubeglass: a read-only terminal viewer for Kubernetes clusters"
)]
pub struct Args {
    /// Path to the kubeconfig file (default: $KUBECONFIG, then ~/.kube/config).
    #[arg(long, value_name = "PATH")]
    pub kubeconfig: Option<PathBuf>,

    /// Kubeconfig context to use (default: current-context).
    #[arg(long, value_name = "NAME")]
    pub context: Option<String>,

    /// Namespace to show (default: the context's namespace, then "default").
    #[arg(
        short = 'n',
        long,
        value_name = "NAME",
        conflicts_with = "all_namespaces"
    )]
    pub namespace: Option<String>,

    /// Show resources from all namespaces.
    #[arg(short = 'A', long)]
    pub all_namespaces: bool,

    /// Disable colours (also enabled by a non-empty NO_COLOR variable).
    #[arg(long)]
    pub no_color: bool,

    /// Show built-in sample data instead of connecting to a cluster.
    #[arg(long)]
    pub demo: bool,

    /// Show local containers instead of a cluster, using a Docker-compatible CLI
    /// (`docker` by default; also `podman`, `nerdctl`). `-n` filters by compose project.
    #[arg(
        long,
        value_name = "COMMAND",
        num_args = 0..=1,
        default_missing_value = "docker",
        conflicts_with_all = ["demo", "context", "kubeconfig"]
    )]
    pub oci: Option<String>,
}

/// Compose project names: lowercase letters, digits, `-` and `_`, starting with a
/// letter or digit.
fn is_valid_compose_project(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

impl Args {
    /// Reject flag combinations this version cannot honour yet.
    pub fn check(&self) -> Result<(), String> {
        if let Some(project) = self.namespace.as_ref().filter(|_| self.oci.is_some()) {
            if !is_valid_compose_project(project) {
                return Err(format!(
                    "\"{project}\" is not a valid compose project name (lowercase letters, digits, '-' and '_')"
                ));
            }
            return Ok(());
        }
        if let Some(ns) = &self.namespace
            && !crate::cluster::config::is_valid_namespace(ns)
        {
            return Err(format!(
                "\"{ns}\" is not a valid namespace name (lowercase letters, digits and '-')"
            ));
        }
        Ok(())
    }

    /// Startup options; `no_color_env` is the value of `NO_COLOR`, if set.
    pub fn to_options(&self, no_color_env: Option<&str>) -> AppOptions {
        AppOptions {
            plain: self.no_color || no_color_env.is_some_and(|v| !v.is_empty()),
            context: self.context.clone(),
            namespace: self.namespace.clone(),
            all_namespaces: self.all_namespaces,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kubectl_style_flags() {
        let a =
            Args::try_parse_from(["kglass", "-n", "payments", "--context", "dev-east"]).unwrap();
        assert_eq!(a.namespace.as_deref(), Some("payments"));
        assert_eq!(a.context.as_deref(), Some("dev-east"));
        assert!(
            Args::try_parse_from(["kglass", "-A"])
                .unwrap()
                .all_namespaces
        );
        assert!(
            Args::try_parse_from(["kglass", "--kubeconfig", "/tmp/k.yaml"])
                .unwrap()
                .kubeconfig
                .is_some()
        );
    }

    #[test]
    fn namespace_and_all_namespaces_conflict() {
        assert!(Args::try_parse_from(["kglass", "-n", "x", "-A"]).is_err());
    }

    #[test]
    fn no_color_env() {
        let a = Args::try_parse_from(["kglass"]).unwrap();
        assert!(a.to_options(Some("1")).plain);
        assert!(!a.to_options(Some("")).plain);
        assert!(!a.to_options(None).plain);
        let b = Args::try_parse_from(["kglass", "--no-color"]).unwrap();
        assert!(b.to_options(None).plain);
    }

    #[test]
    fn demo_flag() {
        assert!(Args::try_parse_from(["kglass", "--demo"]).unwrap().demo);
        assert!(!Args::try_parse_from(["kglass"]).unwrap().demo);
    }

    #[test]
    fn all_namespaces_accepted() {
        let a = Args::try_parse_from(["kglass", "-A"]).unwrap();
        assert!(a.check().is_ok());
        assert!(a.to_options(None).all_namespaces);
    }

    #[test]
    fn invalid_namespace_rejected() {
        let a = Args::try_parse_from(["kglass", "-n", "x/secrets"]).unwrap();
        assert!(a.check().unwrap_err().contains("not a valid namespace"));
    }

    #[test]
    fn oci_flag_defaults_to_docker_and_accepts_a_command() {
        assert_eq!(
            Args::try_parse_from(["kglass", "--oci"])
                .unwrap()
                .oci
                .as_deref(),
            Some("docker")
        );
        assert_eq!(
            Args::try_parse_from(["kglass", "--oci", "podman"])
                .unwrap()
                .oci
                .as_deref(),
            Some("podman")
        );
        assert_eq!(Args::try_parse_from(["kglass"]).unwrap().oci, None);
        let with_project = Args::try_parse_from(["kglass", "--oci", "-n", "shop"]).unwrap();
        assert_eq!(
            (
                with_project.oci.as_deref(),
                with_project.namespace.as_deref()
            ),
            (Some("docker"), Some("shop"))
        );
        for conflicting in [
            ["kglass", "--oci", "--demo"],
            ["kglass", "--oci", "--context=x"],
        ] {
            assert!(
                Args::try_parse_from(conflicting).is_err(),
                "{conflicting:?}"
            );
        }
    }

    #[test]
    fn oci_projects_follow_compose_naming() {
        assert!(
            Args::try_parse_from(["kglass", "--oci", "-n", "my_app"])
                .unwrap()
                .check()
                .is_ok()
        );
        assert!(
            Args::try_parse_from(["kglass", "--oci", "-n", "x/y"])
                .unwrap()
                .check()
                .is_err()
        );
        assert!(
            Args::try_parse_from(["kglass", "-n", "my_app"])
                .unwrap()
                .check()
                .is_err(),
            "kube rules unchanged"
        );
    }

    #[test]
    fn clap_definition_is_valid() {
        use clap::CommandFactory;
        Args::command().debug_assert();
    }
}
