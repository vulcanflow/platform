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

Apply the reviewed revision, not a working copy. Set `commit` to its full
commit id first:

```sh
commit='full-commit-id-of-the-reviewed-revision'
kubectl apply -k "https://github.com/vulcanflow/platform//.tekton/triggers?ref=$commit"
kubectl -n vf-ci rollout status deploy/el-vf-platform
```

## 2. Webhook secret

One value, used in the cluster Secret `vf-ci/vf-ci-webhook` (key `secret`)
and in the GitHub webhook. It never goes on a command line, into this
repository or into issue text. GitHub compares the exact bytes, so the value
must not end in a newline.

First check whether the Secret already exists. `describe` shows key names and
sizes, never the value:

```sh
kubectl -n vf-ci describe secret vf-ci-webhook
```

**It exists.** Do not generate a new value; the webhook must carry the value
the Secret already holds. Under `Data`, key `secret` must show `64 bytes`
(the length of `openssl rand -hex 32`). No `secret` key, or `65 bytes` (a
trailing newline), means no signature can ever match: rotate (below).
Otherwise go on to step 3, and in step 4 enter the value the Secret was
created from.

**It does not exist** (`NotFound`). Generate the value and create the Secret.
If step 1 has not run yet, create the namespace first
(`kubectl create namespace vf-ci`); step 1 then adopts it.

```sh
secret_file=$(mktemp "${TMPDIR:-/tmp}/vf-ci-webhook.XXXXXX")    # mode 0600
openssl rand -hex 32 | tr -d '\n' > "$secret_file"
kubectl -n vf-ci create secret generic vf-ci-webhook \
  --from-file=secret="$secret_file"
```

Keep `$secret_file` until the webhook is registered in step 4; that step reads
and then deletes it. If the shell is lost before then, the variable is gone but
the file is not: rotate, and delete the leftover `vf-ci-webhook.*` file.

**Rotate** when the value is lost, the key or size is wrong, or the value may
have leaked. Do not decode the value back out of the cluster Secret. Generate a
new `$secret_file` as above, replace the Secret in place, then do step 4 with
the new value (edit the existing webhook's Secret field rather than adding a
webhook):

```sh
kubectl -n vf-ci create secret generic vf-ci-webhook \
  --from-file=secret="$secret_file" --dry-run=client -o yaml |
  kubectl -n vf-ci replace -f -
```

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

Check it from anywhere (a GET has no body, so the listener answers with an
error that names it):

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

Exactly one webhook may post to this URL. If the organization already has one
for it (Organization settings, Webhooks), delete that one when you add the
repository webhook. Left in place, it keeps sending the private repositories'
events, and with the same secret every `vulcanflow/platform` event arrives
twice and starts two runs.

| Field | Value |
| --- | --- |
| Payload URL | `https://zozotk.go.ro/vulcanflow/platform` |
| Content type | `application/json` (the filters and bindings read a JSON body) |
| Secret | the value in the cluster Secret: the contents of `$secret_file` from step 2 (`cat "$secret_file"`), or, if the Secret already existed, the value it was created from |
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

The listener answers 202 to every delivery it accepts, including one whose
signature does not match, so neither GitHub nor the response body shows a
wrong secret. After the first push or pull-request delivery, check the
listener log; a match logged for that delivery means the webhook and the Secret
hold different values, or the key is wrong (step 2):

```sh
kubectl -n vf-ci logs deploy/el-vf-platform | grep -iE 'signature|secret'
```

**Why SSL verification is off.** The Gateway presents its `*.zozoo.io`
certificate, which does not cover `zozotk.go.ro`, so GitHub cannot verify the
connection. The owner chose to keep verification off on 2026-10-08 (VFL-124),
knowing the two ways to turn it on below. Deliveries still travel over TLS, and
the HMAC signature still authenticates each one. With verification off, someone
who can intercept traffic between GitHub and the cluster can read deliveries
(the repository is public, so they hold nothing secret) and replay a captured
delivery. A replay re-runs CI for a commit that already exists; without the
secret nobody can forge a delivery or alter one.

To turn verification on, the Payload URL needs a name that the presented
certificate covers: a name under `zozoo.io` (a CNAME to `zozotk.go.ro`, which
the existing wildcard certificate matches), or a certificate issued for
`zozotk.go.ro` itself (for example Let's Encrypt HTTP-01 on port 80). Either
one changes shared DNS or Gateway configuration, and the route from step 3
must accept the new name. Then edit the webhook: the new Payload URL, SSL
verification **Enable**. The Secret and the listener stay as they are.

## Check

A pull request delivery became a run. Set `delivery_id` to GitHub's
X-GitHub-Delivery, shown under the webhook's Recent Deliveries:

```sh
delivery_id='delivery-id-from-recent-deliveries'
kubectl -n vf-ci get pipelineruns -l "vulcanflow.io/github-delivery=$delivery_id" \
  -L triggers.tekton.dev/triggers-eventid
# EventListener lines for that delivery, by the event id on the run's label
event_id=$(kubectl -n vf-ci get pipelineruns \
  -l "vulcanflow.io/github-delivery=$delivery_id" \
  -o jsonpath='{.items[0].metadata.labels.triggers\.tekton\.dev/triggers-eventid}')
kubectl -n vf-ci logs deploy/el-vf-platform |
  grep -F "${event_id:?no run carries that delivery id}"
```

An unsigned POST is refused by the github interceptor and creates no run. The
response carries the listener's `eventID`; the log names it, and no run
carries it:

```sh
event_id=$(curl -sk -X POST -H 'Content-Type: application/json' \
  -H 'X-GitHub-Event: pull_request' -d '{}' \
  https://zozotk.go.ro/vulcanflow/platform |
  sed -n 's/.*"eventID":"\([^"]*\)".*/\1/p')
kubectl -n vf-ci logs deploy/el-vf-platform |
  grep -F "${event_id:?no eventID in the response}"
kubectl -n vf-ci get pipelineruns -l "triggers.tekton.dev/triggers-eventid=$event_id"
# No resources found in vf-ci namespace.
```

`${event_id:?...}` stops the command when the id is empty, where `grep -F ""`
would match every line.

## Roll back

Delete the webhook on `vulcanflow/platform` (and any organization webhook that
posts to `https://zozotk.go.ro/vulcanflow/platform`), remove the rule from the
aether-ci HTTPRoute, then delete this directory's objects by name, the runs
the listener created, and the Secret:

```sh
kubectl -n vf-ci delete eventlisteners.triggers.tekton.dev/vf-platform
kubectl -n vf-ci delete pipelineruns.tekton.dev \
  -l triggers.tekton.dev/eventlistener=vf-platform
kubectl -n vf-ci delete \
  triggertemplates.triggers.tekton.dev/vf-ci-noop \
  triggerbindings.triggers.tekton.dev/vf-github-pull-request \
  triggerbindings.triggers.tekton.dev/vf-github-push \
  pipelines.tekton.dev/vf-ci-noop \
  referencegrants.gateway.networking.k8s.io/aether-ci-httproutes-to-el-vf-platform \
  rolebindings.rbac.authorization.k8s.io/vf-ci-triggers-eventlistener \
  serviceaccounts/vf-ci-triggers \
  secrets/vf-ci-webhook
kubectl delete clusterrolebindings.rbac.authorization.k8s.io/vf-ci-triggers-eventlistener
```

Deleting the EventListener first stops new runs; Kubernetes removes its
Deployment and Service `el-vf-platform` with it. Then remove
`ci/tekton/vulcanflow-platform/github-webhook-secret` from Paperclip's secret
store.

Namespace `vf-ci` is not deleted: it also holds VFL-125's pipelines and any
other VulcanFlow CI objects. Do not use `kubectl delete -k` on this directory
for a roll back, because that deletes `namespace.yaml` and with it everything
in `vf-ci`.
