# Security policy

Kubeglass runs with your Kubernetes credentials, so we take security reports seriously.

## Reporting a vulnerability

Please **do not open a public issue**. Report it privately through the repository's GitHub *Security → Report a vulnerability* page (private security advisories). Include:

- what you found and how to reproduce it
- the kglass version (`kglass --version`) and your platform

You can expect an acknowledgement within a few working days.

**Never include credentials** in a report: no kubeconfig files, tokens or certificates. If output contains secrets, redact them first.

## What kglass promises

- **Read-only.** It only calls `get`, `list` and `pods/log`; see [docs/rbac.md](docs/rbac.md). Any code path that changes cluster state is a security bug.
- **No Secrets.** kglass never reads Secret objects.
- **No credential output.** Tokens, certificates and credential-plugin output are never shown on screen, in errors or in debug output.
- **Terminal safety.** Text from the cluster (names, labels, logs, YAML) is sanitized, so escape sequences cannot control your terminal.
- **No telemetry.** kglass only talks to the API server in your kubeconfig.

A break of any of these is in scope for a report.
