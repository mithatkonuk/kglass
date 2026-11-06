//! `docs/rbac.md` must grant exactly what kglass uses (`fetch::PERMISSIONS`),
//! read-only, and never Secrets.

use std::collections::{BTreeMap, BTreeSet};

use kglass::cluster::fetch::PERMISSIONS;
use serde_json::Value;

/// (kind, apiGroup, resource) → verbs, from every ```yaml block in the doc.
fn granted() -> BTreeMap<(String, String, String), BTreeSet<String>> {
    let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/docs/rbac.md"))
        .expect("docs/rbac.md exists");
    let mut out = BTreeMap::new();
    for block in doc.split("```yaml").skip(1) {
        let yaml = block.split("```").next().unwrap();
        for part in yaml.split("\n---") {
            let obj: Value = serde_norway::from_str(part).expect("valid YAML");
            let kind = obj["kind"].as_str().unwrap_or_default().to_string();
            if kind != "Role" && kind != "ClusterRole" {
                continue;
            }
            for rule in obj["rules"].as_array().expect("rules") {
                let verbs: BTreeSet<String> = rule["verbs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_string())
                    .collect();
                for group in rule["apiGroups"].as_array().unwrap() {
                    for resource in rule["resources"].as_array().unwrap() {
                        out.insert(
                            (
                                kind.clone(),
                                group.as_str().unwrap().to_string(),
                                resource.as_str().unwrap().to_string(),
                            ),
                            verbs.clone(),
                        );
                    }
                }
            }
        }
    }
    out
}

#[test]
fn rbac_doc_matches_code() {
    let granted = granted();
    let wanted: BTreeMap<(String, String, String), BTreeSet<String>> = PERMISSIONS
        .iter()
        .map(|p| {
            let kind = if p.cluster_scoped {
                "ClusterRole"
            } else {
                "Role"
            };
            (
                (
                    kind.to_string(),
                    p.group.to_string(),
                    p.resource.to_string(),
                ),
                p.verbs.iter().map(|v| (*v).to_string()).collect(),
            )
        })
        .collect();
    assert_eq!(
        granted, wanted,
        "docs/rbac.md and fetch::PERMISSIONS differ"
    );
}

#[test]
fn rbac_doc_is_read_only_and_has_no_secrets() {
    for ((_, _, resource), verbs) in granted() {
        assert_ne!(resource, "secrets");
        assert!(resource != "*", "no wildcards");
        for verb in verbs {
            assert!(
                ["get", "list", "watch"].contains(&verb.as_str()),
                "{resource}: {verb}"
            );
        }
    }
}
