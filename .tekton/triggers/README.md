# GitHub webhook to Tekton (VFL-124)

A GitHub webhook on `vulcanflow/platform` posts `pull_request` and `push`
events to the Tekton Triggers EventListener `vf-platform` in namespace `vf-ci`,
reached at `https://zozotk.go.ro`. Every request is signature-checked; a signed
pull request (opened, synchronize, reopened) or branch push creates one
PipelineRun of `vf-ci-noop`, which prints the event and exits.

| File | Objects |
| --- | --- |
| `namespace.yaml` | Namespace `vf-ci` |
| `rbac.yaml` | ServiceAccount `vf-ci-triggers`, its RoleBinding and ClusterRoleBinding |
| `noop-pipeline.yaml` | Pipeline `vf-ci-noop` |
| `trigger-bindings.yaml` | TriggerBindings `vf-github-pull-request`, `vf-github-push` |
| `trigger-template.yaml` | TriggerTemplate `vf-ci-noop` |
| `event-listener.yaml` | EventListener `vf-platform` (Service `el-vf-platform`, port 8080) |

Nothing here touches the GitHub Actions workflows, which keep gating `main`
until Tekton reports the same check names (VFL-126).

## Prerequisites

- Tekton Pipelines serving `tekton.dev/v1` and Tekton Triggers serving
  `triggers.tekton.dev/v1beta1`, with the `github` and `cel`
  ClusterInterceptors and the `tekton-triggers-eventlistener-roles` and
  `tekton-triggers-eventlistener-clusterroles` ClusterRoles that the Triggers
  release installs.
- Rights to create a Namespace and a ClusterRoleBinding (one-time, cluster
  admin); everything else is namespaced to `vf-ci`.

## Apply

```sh
kubectl apply -k .tekton/triggers/

# Webhook secret. The value lives in Paperclip as
# ci/tekton/vulcanflow-platform/github-webhook-secret; pipe it in, never put it
# on the command line or in a file in this repository.
kubectl -n vf-ci create secret generic vf-ci-github-webhook \
  --from-file=secret=/dev/stdin

kubectl -n vf-ci rollout status deploy/el-vf-platform
```

Then route `https://zozotk.go.ro` to Service `el-vf-platform.vf-ci:8080` on the
cluster's ingress, and register the webhook on `vulcanflow/platform`: payload
URL that route, content type `application/json`, secret the same Paperclip
value, events `pull_request` and `push`. Today the host's `/` already serves
another team's EventListener and its certificate does not name
`zozotk.go.ro`; the route and certificate are open on VFL-124 and are not
guessed here.

## Check

```sh
# a pull request event became a run; the delivery id is GitHub's X-GitHub-Delivery
kubectl -n vf-ci get pipelineruns -l vulcanflow.io/github-delivery=<delivery-id>
kubectl -n vf-ci logs deploy/el-vf-platform | grep -i <delivery-id>

# an unsigned POST is refused by the github interceptor and creates no run
curl -sk -X POST -H 'Content-Type: application/json' \
  -H 'X-GitHub-Event: pull_request' -d '{}' <webhook-url>
kubectl -n vf-ci logs deploy/el-vf-platform --since=1m | grep -i signature
```

## Roll back

Delete the webhook on `vulcanflow/platform`, remove the route, then

```sh
kubectl delete -k .tekton/triggers/
```

This removes the namespace with its Secret, any PipelineRuns and anything
else applied to `vf-ci` (VFL-125's pipelines included); nothing outside
`vf-ci` changes except the deleted ClusterRoleBinding.
