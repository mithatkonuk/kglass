//! Relationships derived from metadata. These show configuration, not proof of
//! reachability or health, and the UI labels them `[derived]`.

use std::collections::BTreeMap;

use crate::model::{ImageRef, ImageUsage, Pod};
use crate::text::SafeText;

/// Distinct images used by pods (including init containers), ordered by
/// number of users (descending), then repository (ascending).
pub fn images_in_use(pods: &[Pod]) -> Vec<ImageUsage> {
    let mut by_raw: Vec<ImageUsage> = Vec::new();
    for pod in pods {
        for c in &pod.containers {
            let user = (pod.name.clone(), c.name.clone());
            match by_raw.iter_mut().find(|u| u.raw == c.image) {
                Some(u) => u.users.push(user),
                None => by_raw.push(ImageUsage {
                    raw: c.image.clone(),
                    image: ImageRef::parse(&c.image),
                    users: vec![user],
                }),
            }
        }
    }
    by_raw.sort_by(|a, b| {
        b.users
            .len()
            .cmp(&a.users.len())
            .then_with(|| a.image.repository.cmp(&b.image.repository))
    });
    by_raw
}

/// Pods in `namespace` whose labels contain every selector entry. An empty
/// selector matches nothing (a Service without a selector has manually managed
/// endpoints). Selectors never reach across namespaces.
pub fn pods_matching<'a>(
    selector: &BTreeMap<SafeText, SafeText>,
    namespace: &SafeText,
    pods: &'a [Pod],
) -> Vec<&'a Pod> {
    if selector.is_empty() {
        return Vec::new();
    }
    pods.iter()
        .filter(|p| &p.namespace == namespace)
        .filter(|p| selector.iter().all(|(k, v)| p.labels.get(k) == Some(v)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::sample_snapshot;

    #[test]
    fn images_dedup_and_order() {
        let imgs = images_in_use(&sample_snapshot().pods);
        let rows: Vec<(String, usize)> = imgs
            .iter()
            .map(|i| (i.image.repository.to_string(), i.users.len()))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("envoyproxy/envoy".into(), 2),
                ("registry.acme.io/payments/api".into(), 2),
                ("busybox".into(), 1),
                ("registry.acme.io/payments/billing".into(), 1),
                ("registry.acme.io/payments/ledger-sync".into(), 1),
                ("registry.acme.io/payments/migrate".into(), 1),
                ("registry.acme.io/payments/worker".into(), 1),
            ]
        );
    }

    #[test]
    fn selector_matching() {
        let s = sample_snapshot();
        let billing = s
            .services
            .iter()
            .find(|x| x.name.as_str() == "billing")
            .unwrap();
        assert_eq!(
            pods_matching(&billing.selector, &billing.namespace, &s.pods)
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            ["billing-0"]
        );
        assert!(pods_matching(&BTreeMap::new(), &billing.namespace, &s.pods).is_empty());
        let mut elsewhere = s.pods.clone();
        for p in &mut elsewhere {
            p.namespace = "other-team".into();
        }
        assert!(
            pods_matching(&billing.selector, &billing.namespace, &elsewhere).is_empty(),
            "same labels in another namespace do not match"
        );
    }
}
