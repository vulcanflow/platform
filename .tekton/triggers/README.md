# GitHub webhook to Tekton (VFL-124)

A GitHub webhook on `vulcanflow/platform` posts `pull_request` and `push`
events to `https://zozotk.go.ro/vulcanflow/platform`. The cluster's Gateway
sends that path to the Tekton Triggers EventListener `vf-platform` in
namespace `vf-ci`. Every request is signature-checked; a signed pull request
(opened, synchronize, reopened) or branch push creates one PipelineRun of
`vf-ci-noop`, which prints the event and exits.

| File | Objects |
| --- | --- |
| `namespace.yaml` | Namespace `vf-ci` |
| `rbac.yaml` | ServiceAccount `vf-ci-triggers`, its RoleBinding and ClusterRoleBinding |
| `noop-pipeline.yaml` | Pipeline `vf-ci-noop` |
| `trigger-bindings.yaml` | TriggerBindings `vf-github-pull-request`, `vf-github-push` |
| `trigger-template.yaml` | TriggerTemplate `vf-ci-noop` |
| `event-listener.yaml` | EventListener `vf-platform` (Service `el-vf-platform`, port 8080) |
| `route.yaml` | ReferenceGrant letting the aether-ci HTTPRoute reach `el-vf-platform` |

Nothing here touches the GitHub Actions workflows, which keep gating `main`
until Tekton reports the same check names (VFL-126).

## Prerequisites

- Tekton Pipelines serving `tekton.dev/v1` and Tekton Triggers serving
  `triggers.tekton.dev/v1beta1`, with the `github` and `cel`
  ClusterInterceptors and the `tekton-triggers-eventlistener-roles` and
  `tekton-triggers-eventlistener-clusterroles` ClusterRoles that the Triggers
  release installs.
- Gateway API serving `gateway.networking.k8s.io/v1beta1` ReferenceGrant
  (Cilium's Gateway API support installs it).
- Rights to create a Namespace and a ClusterRoleBinding (one-time, cluster
  admin), and to edit the HTTPRoute in `aether-ci`. Everything else is
  namespaced to `vf-ci`.

## 1. Apply

Apply the reviewed revision, not a working copy:

```sh
kubectl apply -k 'https://github.com/vulcanflow/platform//.tekton/triggers?ref=<commit>'
```

## 2. Webhook secret

One value, used in the cluster Secret `vf-ci/vf-ci-webhook` (key `secret`)
and in the GitHub webhook. It never goes on a command line, into this
repository or into issue text. GitHub compares the exact bytes, so the file
must not end in a newline.

```sh
secret_file=$(mktemp)    # mode 0600
openssl rand -hex 32 | tr -d '\n' > "$secret_file"
kubectl -n vf-ci create secret generic vf-ci-webhook \
  --from-file=secret="$secret_file"
kubectl -n vf-ci rollout status deploy/el-vf-platform
```

Keep `$secret_file` until the webhook is registered in step 4; that step reads
and then deletes it. The Secret can be created before step 1 if `vf-ci` is
created first (`kubectl create namespace vf-ci`); step 1 then adopts the
namespace, and the `rollout status` check runs after step 1.

If the file is lost, rotate rather than decoding the value back out of the
cluster Secret: generate a new value, replace the Secret, and put the new value
in the webhook's Secret field.

## 3. Route

`zozotk.go.ro` is served by the Cilium Gateway `eg`. Its HTTPRoute lives in
namespace `aether-ci` and sends `/` to the aether-ci EventListener. Add this
rule to that HTTPRoute's `spec.rules`, wherever that object is managed:

```yaml
- matches:
    - path:
        type: PathPrefix
        value: /vulcanflow/platform
  backendRefs:
    - name: el-vf-platform
      namespace: vf-ci
      port: 8080
```

The longer prefix wins over `/`, so aether-ci keeps all its other traffic.
The EventListener accepts events on any path, so no rewrite is needed. The
backend is in another namespace; `route.yaml` is the grant that allows it.

Check it from anywhere (a GET has no body, so the listener answers 400 and
names itself):

```sh
curl -sk https://zozotk.go.ro/vulcanflow/platform
# {"eventListener":"vf-platform","namespace":"vf-ci",...}
```

An answer naming `aether-ci` means the rule is not in effect yet. Fix that
before step 4 and before the checks below: until then deliveries and test
requests go to the aether-ci listener.

## 4. Register the webhook

On `vulcanflow/platform`, Settings, Webhooks, Add webhook. Register it on the
repository, not the organization: an organization webhook also sends the
private repositories' events, over the unverified TLS described below.

| Field | Value |
| --- | --- |
| Payload URL | `https://zozotk.go.ro/vulcanflow/platform` |
| Content type | `application/json` (the filters and bindings read a JSON body) |
| Secret | the contents of `$secret_file` from step 2 (`cat "$secret_file"`) |
| SSL verification | **Disable** (see below) |
| Events | Let me select individual events: Pull requests, Pushes |
| Active | on |

Once GitHub has saved the webhook, keep the value in Paperclip's secret store
as `ci/tekton/vulcanflow-platform/github-webhook-secret` for rotation, then
delete the file:

```sh
rm -f "$secret_file"
```

GitHub sends a `ping` first. Both triggers drop it on event type, so the
delivery succeeds and nothing runs. GitHub marks it successful whichever
listener answers, so open the delivery's Response tab: the body must name
`"eventListener":"vf-platform"`. `aether-ci` there means step 3 is not in
effect.

**Why SSL verification is off.** `zozotk.go.ro` is a dynamic-DNS name, so no
certificate can be issued for it; the Gateway presents the `zozoo.io`
certificate. Deliveries still travel over TLS, and the HMAC signature still
authenticates each one. With verification off, someone who can intercept
traffic between GitHub and the cluster can read deliveries (the repository is
public, so they hold nothing secret) and replay a captured delivery. A replay
re-runs CI for a commit that already exists; without the secret nobody can
forge a new delivery.

## Check

```sh
# A pull request delivery became a run. <delivery-id> is GitHub's
# X-GitHub-Delivery, shown under the webhook's Recent Deliveries.
kubectl -n vf-ci get pipelineruns -l vulcanflow.io/github-delivery=<delivery-id> \
  -L triggers.tekton.dev/triggers-eventid
# EventListener lines for that delivery, by the event id from the run's label
kubectl -n vf-ci logs deploy/el-vf-platform | grep <triggers-eventid>

# An unsigned POST is refused by the github interceptor and creates no run.
# The response carries the listener's eventID; grep the log for it.
curl -sk -X POST -H 'Content-Type: application/json' \
  -H 'X-GitHub-Event: pull_request' -d '{}' \
  https://zozotk.go.ro/vulcanflow/platform
kubectl -n vf-ci logs deploy/el-vf-platform | grep <eventID>
kubectl -n vf-ci get pipelineruns
```

## Roll back

Delete the webhook on `vulcanflow/platform`, remove the rule from the
aether-ci HTTPRoute, then

```sh
kubectl delete -k 'https://github.com/vulcanflow/platform//.tekton/triggers?ref=<commit>'
```

This removes the namespace with its Secret, the ReferenceGrant, any
PipelineRuns and anything else applied to `vf-ci` (VFL-125's pipelines
included). Outside `vf-ci`, only the ClusterRoleBinding is deleted.
