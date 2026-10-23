# GitHub webhook to Tekton (VFL-124)

A GitHub webhook on `vulcanflow/platform` posts `pull_request` and `push`
events to `https://zozotk.go.ro/vulcanflow/platform`. The cluster's Gateway
sends that path to the Tekton Triggers EventListener `vf-platform` in
namespace `vf-ci`. Every request is signature-checked.

- A signed pull request (opened, synchronize, reopened) whose head branch is
  in `vulcanflow/platform` creates one PipelineRun of `vf-lane-gate` and one
  of `vf-rust-check` (VFL-777). Both test the pull request's merge ref
  `refs/pull/N/merge`, as GitHub's `actions/checkout` does. A pull request from
  a fork creates nothing.
- A `synchronize` among those, a push to the pull request's branch, also
  creates one PipelineRun of `vf-ci-supersede`. It cancels the pull request's
  unfinished runs for the head the push replaced (VFL-974, "Superseded runs"
  below).
- A signed branch push creates one PipelineRun of `vf-ci-noop`, which prints
  the event and exits.

| File | Objects |
| --- | --- |
| `namespace.yaml` | Namespace `vf-ci` |
| `rbac.yaml` | ServiceAccount `vf-ci-triggers`, its RoleBinding and ClusterRoleBinding; ServiceAccount, Role and RoleBinding `vf-ci-supersede` |
| `noop-pipeline.yaml` | Pipeline `vf-ci-noop` |
| `supersede-pipeline.yaml` | Task and Pipeline `vf-ci-supersede` |
| `trigger-bindings.yaml` | TriggerBindings `vf-github-pull-request`, `vf-github-pull-request-supersede`, `vf-github-push` |
| `trigger-template.yaml` | TriggerTemplate `vf-ci-noop` (pushes) |
| `pull-request-template.yaml` | TriggerTemplate `vf-pull-request` (pull requests) |
| `supersede-template.yaml` | TriggerTemplate `vf-pull-request-supersede` (`synchronize`) |
| `event-listener.yaml` | EventListener `vf-platform` (Service `el-vf-platform`, port 8080) |
| `route.yaml` | ReferenceGrant letting the aether-ci HTTPRoute reach `el-vf-platform` |

The pipelines themselves are in `../` (VFL-125). Their pod settings in
`pull-request-template.yaml` (emptyDir, no service account token, ndots 1,
the arm64 nodeSelector, the timeouts) copy `../pipelineruns/`; keep the two
in step.

Nothing here touches the GitHub Actions workflows, which keep gating `main`
until Tekton reports the same check names (VFL-126). Nothing here reports a
result to GitHub yet either (VFL-778).

## Prerequisites

- A revision at or after the VFL-974 commit. VFL-974 adds the control that
  cancels a pull request's unfinished runs when a newer push supersedes them
  ("Superseded runs"). Without it, every push to a pull request starts seven
  more pods and none stop early. Do not apply `triggers/` from a revision
  without it.
- Tekton Pipelines serving `tekton.dev/v1` and Tekton Triggers serving
  `triggers.tekton.dev/v1beta1`, with the `github` and `cel`
  ClusterInterceptors and the `tekton-triggers-eventlistener-roles` and
  `tekton-triggers-eventlistener-clusterroles` ClusterRoles that the Triggers
  release installs.
- Gateway API serving `gateway.networking.k8s.io/v1beta1` ReferenceGrant
  (Cilium's Gateway API support installs it).
- Rights to create a Namespace and a ClusterRoleBinding (one-time, cluster
  admin), and to edit the HTTPRoute in `aether-ci`. Everything else is
  namespaced to `vf-ci`. The Role `vf-ci-supersede` grants `patch` on
  PipelineRuns, and Kubernetes lets only someone who holds that right grant
  it. The checks in step 4 and under Check also read pod logs in
  `tekton-pipelines`.
- In `vf-ci`, before this revision is applied: the Tasks and Pipelines from
  `../kustomization.yaml`, and the egress policy `vf-ci-check-egress` from
  `../policies/check-egress.yaml`, both applied and tried by hand as in
  `../README.md`, "1. Apply". Apply the policy from the same commit as this
  directory: VFL-974 adds its third rule, which limits the `vf-ci-supersede`
  pods to the Kubernetes API server. From step 1 on, every same-repository
  pull request update starts both pipelines, and their pods run the pull
  request's code. Without the Pipelines the runs fail at once; without the
  policy their pods reach any host. Check all three. The last command prints
  one line per rule of the policy, and one of them must be
  `vf-ci-supersede kube-apiserver`; without it, the policy is from a commit
  before VFL-974:

  ```sh
  kubectl -n vf-ci get pipelines.tekton.dev vf-lane-gate vf-rust-check
  kubectl -n vf-ci get ciliumnetworkpolicies.cilium.io vf-ci-check-egress
  kubectl -n vf-ci get ciliumnetworkpolicies.cilium.io vf-ci-check-egress \
    -o jsonpath='{range .specs[*]}{.endpointSelector.matchLabels.tekton\.dev/pipeline}{" "}{.egress[*].toEntities[*]}{"\n"}{end}'
  ```

  On a cluster without namespace `vf-ci`, create it first
  (`kubectl create namespace vf-ci`); step 1 adopts it.

## 1. Apply

Apply the reviewed revision, not a working copy. Set `commit` to its full
commit id first:

```sh
commit='full-commit-id-of-the-reviewed-revision'
kubectl apply -k "https://github.com/vulcanflow/platform//.tekton/triggers?ref=$commit"
kubectl -n vf-ci rollout status deploy/el-vf-platform
```

The Triggers controller creates Deployment `el-vf-platform` a few seconds
after `apply` returns, so `rollout status` can fail with `NotFound`. Wait a
few seconds and run it again.

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
created from (kept in Paperclip's secret store, see step 4). If you do not
have that value, rotate (below): GitHub never shows a saved webhook secret
again, and the value is not decoded out of the cluster.

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

Register the webhook on the `vulcanflow/platform` repository, not the
organization: an organization webhook also sends the private repositories'
events, over the unverified TLS described below.

Exactly one webhook may post to this URL. With the same secret, a second one
makes every `vulcanflow/platform` event arrive twice and start two runs.
Check both places before adding one:

- Repository (`vulcanflow/platform`, Settings, Webhooks): if a webhook
  already posts to this URL, edit it to the values below instead of adding
  another.
- Organization (Organization settings, Webhooks): if one posts to this URL,
  delete it once the repository webhook is in place. Left in place, it also
  keeps sending the private repositories' events.

Otherwise, on the repository: Settings, Webhooks, Add webhook.

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
wrong secret. After the first push or pull-request delivery, check that a run
carries that delivery's id (X-GitHub-Delivery, under Recent Deliveries):

```sh
delivery_id='delivery-id-from-recent-deliveries'
kubectl -n vf-ci get pipelineruns -l "vulcanflow.io/github-delivery=$delivery_id"
```

No run means an interceptor stopped the delivery. The listener does not log
that at the default level, and the core interceptors log it without any id,
so match their lines to the delivery's time in Recent Deliveries (`ts` is in
Unix seconds):

```sh
kubectl -n tekton-pipelines logs deploy/tekton-triggers-core-interceptors \
  --tail=-1 --since=1h | grep -iE 'signature|secret'
```

`payload signature check failed` at that time means the webhook and the
Secret hold different values, or the Secret has no `secret` key (step 2). The
core interceptors serve every EventListener in the cluster, and the unsigned
POST under Check logs `no X-Hub-Signature-256 header set`, so a line at
another time is not this delivery's.

The log commands here and under Check read the last hour of the current
container's log; widen `--since` for an older delivery. The kubelet rotates
container logs, a restarted container's earlier log needs `--previous`, and a
replaced pod's log is gone. An empty `grep` then means "not in this log", not
"not received"; the run list is the record.

**Why SSL verification is off.** The Gateway presents its `*.zozoo.io`
certificate, which does not cover `zozotk.go.ro`, so GitHub cannot verify the
connection. The owner chose to keep verification off on 2026-10-08 (VFL-124),
knowing the two ways to turn it on below. Deliveries still travel over TLS, and
the HMAC signature still authenticates each one. With verification off, someone
who can intercept traffic between GitHub and the cluster can read deliveries
(the repository is public, so they hold nothing secret) and replay a captured
delivery. A replay re-runs CI for a commit that already exists; without the
secret nobody can forge a delivery or alter one. They can also swallow a
delivery and answer `202` themselves, so GitHub shows it as delivered and no
run exists. With verification on, GitHub would show that delivery as failed.
When Tekton reports a required check (VFL-126), that check must fail closed
when no run exists for a commit, not only when a run fails.

To turn verification on, the Payload URL needs a name that the presented
certificate covers: a name under `zozoo.io` (a CNAME to `zozotk.go.ro`, which
the existing wildcard certificate matches), or a certificate issued for
`zozotk.go.ro` itself (for example Let's Encrypt HTTP-01 on port 80). Either
one changes shared DNS or Gateway configuration, and the route from step 3
must accept the new name. Then edit the webhook: the new Payload URL, SSL
verification **Enable**. The Secret and the listener stay as they are.

## Superseded runs

A push to a pull request's branch sends a `synchronize` delivery. Its `before`
is the head commit the push replaced. Trigger `pull-request-supersede` starts
one `vf-ci-supersede` run for it, next to the two checks for the new head. That
run sets `spec.status: CancelledRunFinally` on every unfinished run whose
`vulcanflow.io/github-pr` is the pull request and whose
`vulcanflow.io/github-head-sha` is `before`, except the runs its own delivery
started. Tekton then stops the checks still running and starts none of the
others. A run that has finished keeps its result. `opened` and `reopened`
deliveries cancel nothing.

The key is the delivery's `before`, not which run is newer. A delivery
redelivered from Recent Deliveries still carries its own `before`, so it names
only the head that it replaced. Like any redelivery, it also starts the two
checks for its own head again; nothing cancels those, and they finish on their
own.

That head can be the current one again, when the branch is pushed back to it.
So the run first takes the time the listener first saw its delivery, from the
oldest run that carries the delivery's id; a redelivery keeps the id. If a run
for `before` from another delivery was created at or after that time (to the
second), the run cancels nothing and logs those runs.

Not covered, and in each case a superseded head's runs are left to finish on
their own:

- A `synchronize` delivery that never reaches the listener cancels nothing,
  because the next push cancels only the head that push replaces. One lost
  delivery leaves one head's runs running.
- GitHub does not promise the order of deliveries. When an older delivery
  reaches the listener after a newer one, or its runs are created after the
  newer delivery's `vf-ci-supersede` run has looked, that run either does not
  see them or takes them for a return to `before`.
- Two pushes in the same second: the check counts a run from the same second
  as newer, so it cancels nothing.

Not covered either, and in each case the current head's runs are cancelled.
Push again, or redeliver from Recent Deliveries the pull request's
`synchronize` delivery whose `after` is the current head, to start new ones:
a delivery's run never cancels the runs that its own delivery started.

- The older delivery's own run, when that delivery reaches the listener after
  a newer one and the newer push went back to the older delivery's `before`.
  Push A to B, then B to A, and the B-to-A delivery arrives first: the A-to-B
  delivery's runs are created after A's, so its run cancels A's runs. Pushing
  B to A and then A to B, with the deliveries arriving at the same times,
  leaves the same runs with the same times, and there cancelling A's runs is
  right. Telling the two apart needs GitHub's own order, which the run cannot
  ask for: it may reach only the Kubernetes API server.
- The check needs the delivery's first runs. If they were deleted, or its
  first attempt never reached the listener, its redelivery after the branch
  returned to `before` cancels the current head's runs.
- A replayed delivery with a new X-GitHub-Delivery (the id is not signed,
  `trigger-template.yaml`) is not caught, as without the check. It cancels the
  runs for its `before`, which can be the current head.

Of the run pods in `vf-ci`, only the `vf-ci-supersede` pod holds a token with
a grant: the `vf-ci-supersede` token. Its Role allows get, list and patch on
PipelineRuns in `vf-ci` and nothing else, and Tekton refuses every spec change
but `spec.status` on a run that has not finished (`rbac.yaml`).
The pod runs none of the revision's code and may reach only the Kubernetes API
server (`../policies/check-egress.yaml`). The run is its own PipelineRun, so
its outcome never changes a check's. It carries no
`vulcanflow.io/github-head-sha`, so it is never taken for a check of a commit.

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
kubectl -n vf-ci logs deploy/el-vf-platform --tail=-1 --since=1h |
  grep -F "${event_id:?no run carries that delivery id}"
```

A pull request delivery starts two runs, `vf-lane-gate-...` and
`vf-rust-check-...`, with seven checks between them. Each check's checkout
step logs the commit it tested as `HEAD <sha> <subject>`. GitHub's merge
commits are titled `Merge <head> into <base>`, so `<head>` there must be the
run's `HEAD-SHA`:

```sh
delivery_id='delivery-id-from-recent-deliveries'
kubectl -n vf-ci get pipelineruns -l "vulcanflow.io/github-delivery=$delivery_id" \
  -L tekton.dev/pipeline,vulcanflow.io/github-pr,vulcanflow.io/github-head-sha
kubectl -n vf-ci get taskruns -l "vulcanflow.io/github-delivery=$delivery_id" \
  -o custom-columns='CHECK:.metadata.labels.tekton\.dev/pipelineTask,STATUS:.status.conditions[0].reason,COMMIT:.status.results[?(@.name=="commit")].value'
kubectl -n vf-ci logs -l "vulcanflow.io/github-delivery=$delivery_id" \
  -c step-checkout --prefix --tail=-1 | grep -F ' HEAD '
```

A different `<head>` means the check fetched a stale merge ref
(`pull-request-template.yaml`), and its result is not that head's. A pull
request from a fork starts no run: its delivery's id matches no
PipelineRun.

The pods must not mount the `vf-ci` `default` token, and must have ndots 1
(`../README.md`, "Prerequisites"). `check-amd64` and `check-arm64` each have
their own pod template in `pull-request-template.yaml`, which Tekton merges
with the run's; this shows that the merge kept both settings:

```sh
kubectl -n vf-ci get pods -l "vulcanflow.io/github-delivery=$delivery_id" \
  -o custom-columns='POD:.metadata.name,PIPELINE:.metadata.labels.tekton\.dev/pipeline,TOKEN:.spec.automountServiceAccountToken,NDOTS:.spec.dnsConfig.options[?(@.name=="ndots")].value'
```

Every check pod must show `false` and `1`. The `vf-ci-supersede` pod of a
`synchronize` delivery shows `true` and no ndots: it mounts the
`vf-ci-supersede` token, and it looks up no name. Across the namespace, no
other pod may show `true`. This finds the pods that turn the token on
themselves. A pod that leaves the field unset shows `<none>` and still mounts
its account's token, as the `vf-ci-noop` pods mount `default`'s, which has no
grant:

```sh
kubectl -n vf-ci get pods \
  -o custom-columns='POD:.metadata.name,PIPELINE:.metadata.labels.tekton\.dev/pipeline,TOKEN:.spec.automountServiceAccountToken' |
  awk 'NR == 1 || $3 == "true"'
```

A `synchronize` delivery starts three runs: the two checks for the new head
and one `vf-ci-supersede-...`. Take `pr` and `before` from that delivery's
payload under Recent Deliveries (`pull_request.number`, `before`). The
supersede run's log names each run it cancelled, and the runs for `before`
show what happened to them:

```sh
delivery_id='delivery-id-of-a-synchronize'
pr='pull-request-number'
before='before-commit-from-the-payload'
kubectl -n vf-ci get pipelineruns -l "vulcanflow.io/github-delivery=$delivery_id" \
  -L tekton.dev/pipeline
kubectl -n vf-ci logs \
  -l "vulcanflow.io/github-delivery=$delivery_id,tekton.dev/pipeline=vf-ci-supersede" \
  -c step-cancel --tail=-1
kubectl -n vf-ci get pipelineruns \
  -l "vulcanflow.io/github-pr=$pr,vulcanflow.io/github-head-sha=$before" \
  -o custom-columns='RUN:.metadata.name,SPEC:.spec.status,STATUS:.status.conditions[0].reason'
```

Each run for `before` that had not finished shows `CancelledRunFinally`, and
its STATUS ends as `Cancelled`, or `Failed` if one of its checks had already
failed. A run that had finished shows no SPEC and keeps its STATUS. An
`opened` or `reopened` delivery starts the two checks and no
`vf-ci-supersede` run. Redelivering an older `synchronize` from Recent
Deliveries cancels only that delivery's `before`, and nothing when the branch
has since been pushed back to `before`; its log then lists the runs for
`before` that came after the delivery. Either way the runs for the current
head keep running, within the limits under Superseded runs.

An unsigned POST is refused by the github interceptor and creates no run. The
response carries the listener's `eventID`; the check is that no run carries
it:

```sh
event_id=$(curl -sk -X POST -H 'Content-Type: application/json' \
  -H 'X-GitHub-Event: pull_request' -d '{}' \
  https://zozotk.go.ro/vulcanflow/platform |
  sed -n 's/.*"eventID":"\([^"]*\)".*/\1/p')
date -u +%s    # the POST time, in Unix seconds like the log's ts
kubectl -n vf-ci get pipelineruns \
  -l "triggers.tekton.dev/triggers-eventid=${event_id:?no eventID in the response}"
# No resources found in vf-ci namespace.
kubectl -n tekton-pipelines logs deploy/tekton-triggers-core-interceptors \
  --tail=-1 --since=1h | grep -F 'no X-Hub-Signature-256 header set'
```

At the default log level no EventListener line names the refused id: Triggers
v0.34.0 logs an interceptor stop at `debug` only. The core interceptors log
the refusal (`Continue:false`, `FailedPrecondition`,
`no X-Hub-Signature-256 header set`) without an event id, so the last command
only supports the check: its line should have a `ts` at the POST time.

`${event_id:?...}` stops the command when the id is empty, where `grep -F ""`
would match every line and an empty label value would match no run, which
reads as a pass.

## Roll back

To stop pull requests from starting the pipelines (VFL-777) and keep the rest,
roll back the supersede control (VFL-974) with it, not on its own: without the
control, every push starts seven more pods and none stop early. Apply the
triggers revision before VFL-777 as in step 1, then delete the objects it no
longer names and the runs they started:

```sh
commit='full-commit-id-of-the-revision-before-VFL-777'
kubectl apply -k "https://github.com/vulcanflow/platform//.tekton/triggers?ref=$commit"
kubectl -n vf-ci delete --ignore-not-found \
  triggertemplates.triggers.tekton.dev/vf-pull-request \
  triggertemplates.triggers.tekton.dev/vf-pull-request-supersede \
  triggerbindings.triggers.tekton.dev/vf-github-pull-request-supersede \
  pipelines.tekton.dev/vf-ci-supersede \
  tasks.tekton.dev/vf-ci-supersede \
  rolebindings.rbac.authorization.k8s.io/vf-ci-supersede \
  roles.rbac.authorization.k8s.io/vf-ci-supersede \
  serviceaccounts/vf-ci-supersede
kubectl -n vf-ci delete --ignore-not-found pipelineruns.tekton.dev \
  -l 'triggers.tekton.dev/eventlistener=vf-platform,tekton.dev/pipeline in (vf-lane-gate,vf-rust-check,vf-ci-supersede)'
```

The label selector leaves the runs started by hand (`../README.md`, "Run by
hand"), which carry no EventListener label.

Pull requests then start `vf-ci-noop` again.

To remove everything: roll back VFL-125 first. The steps below remove the
EventListener, Secret and TriggerTemplates that VFL-125 reuses, and delete
every PipelineRun labelled with this EventListener, VFL-125's included.

Delete the webhook on `vulcanflow/platform` (and any organization webhook that
posts to `https://zozotk.go.ro/vulcanflow/platform`), remove the rule from the
aether-ci HTTPRoute, then delete this directory's objects by name, the runs
the listener created, and the Secret. `--ignore-not-found` lets a partly
installed slice roll back without `NotFound` errors:

```sh
kubectl -n vf-ci delete --ignore-not-found \
  eventlisteners.triggers.tekton.dev/vf-platform
kubectl -n vf-ci delete --ignore-not-found pipelineruns.tekton.dev \
  -l triggers.tekton.dev/eventlistener=vf-platform
kubectl -n vf-ci delete --ignore-not-found \
  triggertemplates.triggers.tekton.dev/vf-ci-noop \
  triggertemplates.triggers.tekton.dev/vf-pull-request \
  triggertemplates.triggers.tekton.dev/vf-pull-request-supersede \
  triggerbindings.triggers.tekton.dev/vf-github-pull-request \
  triggerbindings.triggers.tekton.dev/vf-github-pull-request-supersede \
  triggerbindings.triggers.tekton.dev/vf-github-push \
  pipelines.tekton.dev/vf-ci-noop \
  pipelines.tekton.dev/vf-ci-supersede \
  tasks.tekton.dev/vf-ci-supersede \
  referencegrants.gateway.networking.k8s.io/aether-ci-httproutes-to-el-vf-platform \
  rolebindings.rbac.authorization.k8s.io/vf-ci-triggers-eventlistener \
  rolebindings.rbac.authorization.k8s.io/vf-ci-supersede \
  roles.rbac.authorization.k8s.io/vf-ci-supersede \
  serviceaccounts/vf-ci-triggers \
  serviceaccounts/vf-ci-supersede \
  secrets/vf-ci-webhook
kubectl delete --ignore-not-found \
  clusterrolebindings.rbac.authorization.k8s.io/vf-ci-triggers-eventlistener
```

Deleting the EventListener first stops new runs; Kubernetes removes its
Deployment and Service `el-vf-platform` with it. Then remove
`ci/tekton/vulcanflow-platform/github-webhook-secret` from Paperclip's secret
store.

Namespace `vf-ci` is not deleted: it is shared with VFL-125 and any other
VulcanFlow CI objects. Its Pod Security labels from step 1 stay on it
by design; they apply to every pod in `vf-ci`, not only to this slice's. Do
not use `kubectl delete -k` on this directory for a roll back, because that
deletes `namespace.yaml` and with it everything in `vf-ci`.
