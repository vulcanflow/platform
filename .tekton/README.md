# CI pipelines on Tekton (VFL-125, VFL-716)

Tekton Tasks and Pipelines that reproduce, step for step, the GitHub Actions
lane gate and both legs of rust-check. The amd64 leg is cross-built on an
arm64 node (see "Architectures"). They run in namespace `vf-ci` next to the
webhook listener from `triggers/` (VFL-124). The webhook starts both for
every pull request from a branch of this repository (`triggers/README.md`,
VFL-777); they can also be started by hand. Reporting their result to the
pull request is later work (see "Not here yet"). The GitHub Actions workflows
stay in place until Tekton reports the same contexts.

| GitHub job (status context) | Workflow | PipelineTask | Task | Command |
|---|---|---|---|---|
| `lane-partition` | `lane-gate.yml` | `lane-partition` in `vf-lane-gate` | `vf-lane-gate` | `ci/lane-gate.sh partition` |
| `test-erosion` | `lane-gate.yml` | `test-erosion` in `vf-lane-gate` | `vf-lane-gate` | `ci/lane-gate.sh erosion` |
| `inline-test-modules` | `lane-gate.yml` | `inline-test-modules` in `vf-lane-gate` | `vf-lane-gate` | `ci/lane-gate.sh inline-tests` |
| `uses-pins` | `lane-gate.yml` | `uses-pins` in `vf-lane-gate` | `vf-lane-gate` | `ci/lane-gate.sh pins` |
| `gate-self-test` | `lane-gate.yml` | `gate-self-test` in `vf-lane-gate` | `vf-lane-gate-selftest` | `ci/lane-gate-test.sh ci/lane-gate.sh` |
| `check (arm64)` | `rust-check.yml` | `check-arm64` in `vf-rust-check` | `vf-rust-check` | `just check`, `just test-unit`, `just fuzz-check` |
| `check (amd64)` | `rust-check.yml` | `check-amd64` in `vf-rust-check` | `vf-rust-check` (`execution: cross-qemu`) | the same |

The pass/fail logic is in the scripts and `just` recipes, which both executors
call with the same command line. Each Task header lists its deliberate
differences from the workflow, none of which can change a verdict. In short:
images and tools are pinned by digest, steps run as an unprivileged user,
and there is no build cache.

## Files

- `kustomization.yaml`: the three Tasks and two Pipelines. `triggers/` is
  separate.
- `tasks/`, `pipelines/`: the definitions.
- `pipelineruns/`: one manual PipelineRun per pipeline, with an `@REVISION@`
  placeholder. Not part of the kustomization.
- `policies/check-egress.yaml`: the egress policy on the check pods (see
  "Prerequisites"). Not part of the kustomization either; it is applied on its
  own.

## Architectures

All four zolab nodes are arm64 (2026-10-13), so `check (arm64)` runs natively
on a worker node. Its entry in `taskRunSpecs` in `pipelineruns/rust-check.yaml`
sets `nodeSelector: kubernetes.io/arch: arm64`, a label the kubelet puts on
every node. The Task compares `uname -m` with its `arch` parameter and fails on
a mismatch, so the leg can never report green from the wrong architecture.
Check the nodes with:

```sh
kubectl get nodes -L kubernetes.io/arch
```

`check (amd64)` cannot run natively here. Cortana decided on 2026-10-13
(VFL-125) to cross-build it on an arm64 node and run its unit tests under
user-mode QEMU in an unprivileged pod (VFL-716). Its `taskRunSpecs` entry
therefore also selects arm64, and the Pipeline passes `execution: cross-qemu`.
In that mode the Task requires an arm64 pod and `arch: amd64`. It unpacks a
pinned x86_64 cross gcc and g++ and `qemu-x86_64-static` from Debian bookworm
under /tmp. It sets `CARGO_BUILD_TARGET=x86_64-unknown-linux-gnu`, and cargo
runs every unit-test binary through qemu. Nothing is installed on the node,
the pod is not privileged, and no binfmt_misc handler is registered. The
header of `tasks/rust-check.yaml` lists what this still does not reproduce
from a native amd64 runner. If an amd64 node joins later, the leg can run
natively instead: `execution: native` in the Pipeline and
`kubernetes.io/arch: amd64` in its `taskRunSpecs` entry.

GitHub's `rust-check.yml` keeps running both legs until the workflows are
retired (VFL-127).

## Prerequisites

- Namespace `vf-ci`, from a `triggers/` revision before VFL-777 or from
  `kubectl create namespace vf-ci`. `triggers/` comes last, after step 2, only
  at or after the VFL-974 commit, and from the same commit as the egress
  policy: it starts these pipelines for every pull request update
  (`triggers/README.md`, "Prerequisites").
- `kubectl` with rights to create Tasks, Pipelines and PipelineRuns in `vf-ci`.
- Tekton Pipelines with the `tekton.dev/v1` API. The cluster had v1.15.1 on
  2026-10-09.
- Egress from the pods to `github.com`,
  `release-assets.githubusercontent.com`, `static.rust-lang.org`,
  `index.crates.io`, `static.crates.io` and `deb.debian.org`. All six
  release downloads in `tasks/rust-check.yaml` (both architectures) redirect
  from `github.com` to `release-assets.githubusercontent.com` (measured
  2026-10-13). If one redirects elsewhere, that host is added here and in the
  policy with the reason. Only `check (amd64)` uses `deb.debian.org`, for its
  cross packages; it serves them without a redirect (2026-10-14). The node
  pulls the images from Docker Hub, not the pod.
- Before the webhook starts these pipelines, the default-deny egress policy
  in `policies/check-egress.yaml`, a `CiliumNetworkPolicy`
  `vf-ci-check-egress` in `vf-ci`. The pods run code from the revision under
  test (build scripts, proc macros, tests). They hold no credential: no Secret
  is mounted, and the PipelineRuns that start them (`pipelineruns/` and
  `triggers/pull-request-template.yaml`) set
  `automountServiceAccountToken: false`. A Pipeline cannot set that, so a run
  created any other way mounts the token. Without the policy the pods reach
  any host and the whole cluster network. It selects the pods whose
  `tekton.dev/pipeline` label is `vf-lane-gate` or `vf-rust-check`, and allows
  egress only:
  - to the hosts in the bullet above, by `toFQDNs` `matchName`, on TCP port
    443. Only the `check-amd64` pods may reach `deb.debian.org`: a second
    rule selects them by their `tekton.dev/pipelineTask` label;
  - to cluster DNS, the `k8s-app: kube-dns` pods in `kube-system` on port 53
    UDP and TCP, with a DNS rule (`rules.dns`) that allows lookups of the same
    hosts by `matchName`, and of no other name.

  A third rule selects the `vf-ci-supersede` pods (`triggers/`, VFL-974). They
  run none of the revision's code but hold a token that may patch
  PipelineRuns, so they may reach the Kubernetes API server
  (`toEntities: kube-apiserver`) and nothing else.

  Once an egress rule selects a pod, Cilium denies all other egress from it.
  Host names need Cilium: for destinations outside the cluster a Kubernetes
  `NetworkPolicy` can only match IP blocks, and the addresses behind these
  hosts change. Cilium's DNS proxy reads the pods' DNS answers and allows the
  addresses they return, so a `toFQDNs` rule matches nothing without the DNS
  rule. The proxy refuses a lookup of any other name, so a pod cannot pass
  data out in the names it looks up (VFL-948). That includes the
  search-domain forms of a host, such as `github.com.vf-ci.svc.cluster.local`.
  With the kubelet's default `ndots:5` a pod looks those up first. musl stops
  at the refusal and never looks up the name as given. It is the resolver
  inside the static `cargo-deny` binary on every node, and inside
  `cargo-audit` on an amd64 node, so `cargo deny check` could not fetch its
  advisory database (VFL-982). So the PipelineRuns also set `ndots: 1` in
  their `dnsConfig`. Every listed host has a dot, so glibc and musl both look
  it up as given first, and no search-domain form is sent. The checks
  therefore do not depend on the response code the proxy refuses with. A
  `vf-rust-check` run created any other way keeps `ndots:5`, so the `deny`
  recipe of `just check`, in the `check` step, fails and the run does not
  pass. That holds unless the run sets its own `dnsConfig`, or the cluster
  sets one in Tekton's `default-pod-template`. The owner confirmed on
  2026-10-13 that the cluster's Gateway `eg` is Cilium's, so the CNI is
  Cilium. The EventListener pod is not selected; it has no policy either
  (VFL-124 LOW-4).

  The policy assumes two things about the cluster. Check both, read-only,
  before applying it:

  ```sh
  kubectl -n kube-system get configmap cilium-config -o jsonpath='{.data.enable-l7-proxy}{"\n"}'
  kubectl -n kube-system get pods -l k8s-app=kube-dns -o wide
  ```

  The first must print `true`: Cilium's DNS proxy is part of its L7 proxy, and
  `toFQDNs` needs it. The second must list the cluster's DNS pods. If either
  fails, the policy denies all egress and every check fails at its first
  `git fetch`.

## 1. Apply

`$commit` is the commit of this directory to apply, normally the PR head:

```sh
commit=<commit>; kubectl apply -k "https://github.com/vulcanflow/platform//.tekton?ref=$commit"
```

This creates or updates three Tasks (`vf-lane-gate`, `vf-lane-gate-selftest`,
`vf-rust-check`) and two Pipelines (`vf-lane-gate`, `vf-rust-check`) in
`vf-ci`. It does not touch `triggers/`.

Then, once both checks under "Prerequisites" pass, the egress policy. Apply
it from the commit you will apply `triggers/` from, so that it carries the
rule for the `vf-ci-supersede` pods that `triggers/` adds:

```sh
commit=<commit>; kubectl apply -f "https://raw.githubusercontent.com/vulcanflow/platform/$commit/.tekton/policies/check-egress.yaml"
```

Then check the DNS rule. This starts a pod from the check image, with the
label the policy selects, no token and `ndots:1`, and looks up one listed
host and one other host. Pod Security prints a `restricted` warning; `vf-ci`
enforces only `baseline`:

```sh
image=$(kubectl -n vf-ci get pipeline vf-lane-gate -o jsonpath='{.spec.params[?(@.name=="image")].default}'); kubectl -n vf-ci run vf-dns-probe --rm -i --restart=Never --labels=tekton.dev/pipeline=vf-lane-gate --image="$image" --overrides='{"apiVersion":"v1","spec":{"automountServiceAccountToken":false,"dnsConfig":{"options":[{"name":"ndots","value":"1"}]}}}' -- sh -c 'grep ndots /etc/resolv.conf; getent hosts github.com; echo "github.com: $?"; getent hosts example.com; echo "example.com: $?"'
```

It must print `options ndots:1`, an address for `github.com` and
`github.com: 0`, then `example.com: 2` and no address. An address for
`example.com` means some other policy allows these pods port 53 with no DNS
rule, which Cilium merges into a lookup of any name. Find that policy before
going on.

`getent` resolves through glibc, so the probe does not cover musl. Step 2
does: the `deny` recipe of `just check`, in the `check` step of both
`vf-rust-check` legs, runs the musl `cargo-deny`, which looks up
`github.com`. Start both pipelines by hand once (step 2) with the policy in
place. Both must pass before `triggers/` starts them from the webhook; a host
the policy lacks shows as a failed fetch in the `check` step's log.

## 2. Run by hand

`$commit` is the same as in step 1, and `$rev` is the full 40-character SHA
under test. For a pull request, `$rev` is the merge commit GitHub's own run
checked out: its checkout step prints it on the line after
`git log -1 --format=%H`.

```sh
commit=<commit>; rev=<sha>; curl -fsSL "https://raw.githubusercontent.com/vulcanflow/platform/$commit/.tekton/pipelineruns/lane-gate.yaml" | sed "s|@REVISION@|$rev|" | kubectl create -f -
```

```sh
commit=<commit>; rev=<sha>; curl -fsSL "https://raw.githubusercontent.com/vulcanflow/platform/$commit/.tekton/pipelineruns/rust-check.yaml" | sed "s|@REVISION@|$rev|" | kubectl create -f -
```

Each prints the run name, e.g. `pipelinerun.tekton.dev/vf-lane-gate-x7k2p
created`.

## 3. Read the result

`$run` is the name printed in step 2. This prints the run's status and
duration, one row per check with its result, the commit it ran on and the base
tip it compared with, then the last 40 log lines of every container (each
step, and Tekton's own init containers):

```sh
run=<name>; kubectl -n vf-ci get pipelinerun "$run" -o custom-columns='RUN:.metadata.name,STATUS:.status.conditions[0].reason,START:.status.startTime,END:.status.completionTime'; echo ---; kubectl -n vf-ci get taskruns -l "tekton.dev/pipelineRun=$run" -o custom-columns='CHECK:.metadata.labels.tekton\.dev/pipelineTask,STATUS:.status.conditions[0].reason,START:.status.startTime,END:.status.completionTime,COMMIT:.status.results[?(@.name=="commit")].value,BASE:.status.results[?(@.name=="base-commit")].value'; echo ---; kubectl -n vf-ci logs -l "tekton.dev/pipelineRun=$run" --all-containers --prefix --tail=40
```

A check passed when its STATUS is `Succeeded` and failed when it is `Failed`.
The pipeline's own STATUS is `Failed` when any one check failed, as a GitHub
workflow run is.

## Comparing with GitHub

Both executors give the same verdict when they see the same inputs. Three
inputs change over time, so compare runs started close together:

- **The base tip.** `test-erosion` counts tests at the `git merge-base` of the
  tip of `main` as fetched and the checked-out revision, so a tip that moved
  ahead between the two runs does not change the base count. The `BASE` column
  shows the tip Tekton fetched.
- **The advisory databases.** `just check` runs cargo-deny and cargo-audit
  against the latest RustSec data. A new advisory turns every commit red on
  both executors from the moment it is published.
- **`just`.** The workflow installs the latest release; the Task pins 1.58.0.
  If `just` publishes a new release, update the pin and digests in
  `tasks/rust-check.yaml`.

Two more apply to `check (amd64)` only:

- **Emulation.** Its build scripts run on arm64 and its unit tests run under
  qemu. A failure on this leg alone that GitHub's amd64 leg does not show can
  come from that. The header of `tasks/rust-check.yaml` lists what to look for.
- **The Debian pool.** The cross packages are fetched by version from
  `deb.debian.org`. When Debian replaces one of them in bookworm, the old file
  leaves the pool and the leg fails at its download with a 404, before any
  check runs. That is a missing check, not a verdict. Regenerate the list as
  that header describes.

## Clean up

The manual runs stay until deleted. Deleting a PipelineRun deletes its
TaskRuns and pods. The selector skips the runs the webhook started, which
carry the EventListener's label and may still be running for a pull request:

```sh
kubectl -n vf-ci delete pipelinerun -l 'tekton.dev/pipeline in (vf-lane-gate,vf-rust-check),!triggers.tekton.dev/eventlistener'
```

## Roll back

First stop the webhook from starting these pipelines (`triggers/README.md`,
"Roll back"). Then clean up as above and remove the five definitions by name.
Namespace `vf-ci` and everything from `triggers/` stay:

```sh
kubectl -n vf-ci delete pipeline vf-lane-gate vf-rust-check; kubectl -n vf-ci delete task vf-lane-gate vf-lane-gate-selftest vf-rust-check
```

The egress policy goes last, once nothing starts these pipelines:

```sh
kubectl -n vf-ci delete --ignore-not-found ciliumnetworkpolicies.cilium.io/vf-ci-check-egress
```

## Not here yet

- Runs for pull requests from forks. The webhook starts these pipelines only
  for pull requests whose head branch is in this repository
  (`triggers/event-listener.yaml`); a fork's pull request starts nothing.
- Pruning of finished runs, and a LimitRange or ResourceQuota on `vf-ci`. Each
  pull request update starts seven pods: the five `vf-lane-gate` checks, which
  run for up to 20m, and `check-amd64` and `check-arm64`, which request 2 CPU
  and 4Gi each and run for up to 1h30m. A push cancels its pull request's
  unfinished runs for the head it replaced (`triggers/README.md`, "Superseded
  runs"), but the runs of different pull requests still compete for the
  nodes. A run's timeouts count from its creation, not from when its pods are
  scheduled, so after a burst of updates a check can time out while its pod
  is still `Pending`. That red is not the code's.
- Reporting each check to the PR commit as a status with the same context
  name, and switching the required contexts on `main` (VFL-126).
