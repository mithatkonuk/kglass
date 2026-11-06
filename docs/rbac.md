# Permissions (RBAC)

Kubeglass is **read-only**. It only ever calls `get`, `list`, `pods/log` and the API server's `/version` endpoint (which every authenticated user can read by default), and it **never reads Secrets**. If you can already run `kubectl get pods` and `kubectl logs` in a namespace, you most likely have everything it needs.

This page is for cluster admins who want to grant the minimum. A test (`tests/rbac_doc.rs`) checks that the YAML below grants exactly the permission list the code declares (`fetch::PERMISSIONS`). That list is maintained by hand next to the API calls in `src/cluster/fetch.rs`.

## Namespace access (required)

Grant this in each namespace people should be able to view:

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: kglass-viewer
  namespace: payments
rules:
  - apiGroups: [""]
    resources: ["pods", "services", "persistentvolumeclaims"]
    verbs: ["get", "list"]
  - apiGroups: [""]
    resources: ["pods/log"]
    verbs: ["get"]
  - apiGroups: [""]
    resources: ["events"]
    verbs: ["list"]
  - apiGroups: ["discovery.k8s.io"]
    resources: ["endpointslices"]
    verbs: ["list"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: kglass-viewer
  namespace: payments
subjects:
  - kind: Group
    name: developers
    apiGroup: rbac.authorization.k8s.io
roleRef:
  kind: Role
  name: kglass-viewer
  apiGroup: rbac.authorization.k8s.io
```

Without `events`, the Events tabs stay empty. Everything else needs the rules above.

## Cluster-scoped extras (optional)

PersistentVolumes and the namespace list are cluster-scoped. Without these rules, kglass still works:

- The Storage pane shows the claims with a note that PVs are hidden.
- The namespace picker (`n`) lets you type a namespace name instead of picking one from a list.

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: kglass-cluster-viewer
rules:
  - apiGroups: [""]
    resources: ["persistentvolumes"]
    verbs: ["get", "list"]
  - apiGroups: [""]
    resources: ["namespaces"]
    verbs: ["list"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRoleBinding
metadata:
  name: kglass-cluster-viewer
subjects:
  - kind: Group
    name: developers
    apiGroup: rbac.authorization.k8s.io
roleRef:
  kind: ClusterRole
  name: kglass-cluster-viewer
  apiGroup: rbac.authorization.k8s.io
```

## All namespaces (`-A`)

`kglass -A` lists pods, services, claims, EndpointSlices and events across the whole cluster. That needs the namespace rules above granted cluster-wide: put the same rules in a ClusterRole and bind it with a ClusterRoleBinding. If a list is not allowed cluster-wide, that pane shows `Forbidden (403)` with the exact rule needed.

## Checking your access

```bash
kubectl auth can-i list pods
kubectl auth can-i get pods --subresource=log
kubectl auth can-i list persistentvolumes
```
