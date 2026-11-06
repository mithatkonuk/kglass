//! The GitHub workflows only run once a remote exists, so check their shape here.

use serde_json::Value;

fn workflow(name: &str) -> (String, Value) {
    let path = format!("{}/.github/workflows/{name}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).expect("workflow exists");
    let value = serde_norway::from_str(&text).expect("valid YAML");
    (text, value)
}

#[test]
fn ci_runs_the_local_quality_gate() {
    let (text, ci) = workflow("ci.yml");
    for command in [
        "cargo fmt --check",
        "cargo clippy --locked --all-targets -- -D warnings",
        "cargo test --locked",
    ] {
        assert!(text.contains(command), "CI must run `{command}`");
    }
    assert_eq!(ci["permissions"]["contents"], "read", "least privilege");
    let msrv = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
    assert!(msrv.contains("rust-version = \"1.89\""));
    assert!(
        text.contains("dtolnay/rust-toolchain@1.89"),
        "CI builds on the declared MSRV"
    );
}

#[test]
fn release_builds_four_targets_on_version_tags() {
    let (text, release) = workflow("release.yml");
    assert_eq!(release["on"]["push"]["tags"][0], "v*");
    let targets: Vec<&str> = release["jobs"]["build"]["strategy"]["matrix"]["include"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["target"].as_str().unwrap())
        .collect();
    assert_eq!(
        targets,
        [
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
            "x86_64-unknown-linux-gnu",
            "aarch64-unknown-linux-gnu"
        ]
    );
    assert!(text.contains("--locked --release"), "reproducible builds");
    assert!(text.contains("shasum -a 256"), "checksums are published");
}
