# CI pipelines on Tekton (VFL-125)

Tekton Tasks and Pipelines that reproduce, step for step, the GitHub Actions
lane gate and the arm64 leg of rust-check. The amd64 leg follows (see
"Architectures"). They run in namespace `vf-ci` next to the webhook listener
from `triggers/` (VFL-124). For now they are started by hand. Starting them
from the webhook and reporting their result to the pull request is later work
(see "Not here yet"). The GitHub Actions workflows stay in place until Tekton
reports the same contexts.

| GitHub job (status context) | Workflow | PipelineTask | Task | Command |
|---|---|---|---|---|
| `lane-partition` | `lane-gate.yml` | `lane-partition` in `vf-lane-gate` | `vf-lane-gate` | `ci/lane-gate.sh partition` |
| `test-erosion` | `lane-gate.yml` | `test-erosion` in `vf-lane-gate` | `vf-lane-gate` | `ci/lane-gate.sh erosion` |
| `inline-test-modules` | `lane-gate.yml` | `inline-test-modules` in `vf-lane-gate` | `vf-lane-gate` | `ci/lane-gate.sh inline-tests` |
| `gate-self-test` | `lane-gate.yml` | `gate-self-test` in `vf-lane-gate` | `vf-lane-gate-selftest` | `ci/lane-gate-test.sh ci/lane-gate.sh` |
| `check (arm64)` | `rust-check.yml` | `check-arm64` in `vf-rust-check` | `vf-rust-check` | `just check`, `just test-unit`, `just fuzz-check` |
| `check (amd64)` | `rust-check.yml` | not on Tekton yet (VFL-716) | | the same |

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
user-mode QEMU in an unprivileged pod. That is follow-up VFL-716. Until it
lands, GitHub's `rust-check.yml` keeps running both legs.

## Prerequisites

- `triggers/` applied, so namespace `vf-ci` exists.
- `kubectl` with rights to create Tasks, Pipelines and PipelineRuns in `vf-ci`.
- Tekton Pipelines with the `tekton.dev/v1` API. The cluster had v1.15.1 on
  2026-10-09.
- Egress from the pods to `github.com`, `objects.githubusercontent.com`,
  `static.rust-lang.org`, `index.crates.io` and `static.crates.io`, and to
  Docker Hub for the image.

## 1. Apply

`$commit` is the commit of this directory to apply, normally the PR head:

```sh
commit=<commit>; kubectl apply -k "https://github.com/vulcanflow/platform//.tekton?ref=$commit"
```

This creates or updates three Tasks (`vf-lane-gate`, `vf-lane-gate-selftest`,
`vf-rust-check`) and two Pipelines (`vf-lane-gate`, `vf-rust-check`) in
`vf-ci`. It does not touch `triggers/`.

## 2. Run by hand

`$commit` is the same as in step 1, and `$rev` is the full 40-character SHA
under test. For a pull request, `$rev` is the merge commit GitHub's own run
checked out: its checkout step prints it on the line after
`git log -1 --format=%H`.

```sh
commit=<commit>; rev=<sha>; curl -fsSL "https://raw.githubusercontent.com/vulcanflow/platform/$commit/.tekton/pipelineruns/lane-gate.yaml" | sed "s/@REVISION@/$rev/" | kubectl create -f -
```

```sh
commit=<commit>; rev=<sha>; curl -fsSL "https://raw.githubusercontent.com/vulcanflow/platform/$commit/.tekton/pipelineruns/rust-check.yaml" | sed "s/@REVISION@/$rev/" | kubectl create -f -
```

Each prints the run name, e.g. `pipelinerun.tekton.dev/vf-lane-gate-x7k2p
created`.

## 3. Read the result

`$run` is the name printed in step 2. This prints the run's status and
duration, one row per check with its result, the commit it ran on and the base
tip it compared with, then the last 40 log lines of every step:

```sh
run=<name>; kubectl -n vf-ci get pipelinerun "$run" -o custom-columns='RUN:.metadata.name,STATUS:.status.conditions[0].reason,START:.status.startTime,END:.status.completionTime'; echo ---; kubectl -n vf-ci get taskruns -l "tekton.dev/pipelineRun=$run" -o custom-columns='CHECK:.metadata.labels.tekton\.dev/pipelineTask,STATUS:.status.conditions[0].reason,START:.status.startTime,END:.status.completionTime,COMMIT:.status.results[?(@.name=="commit")].value,BASE:.status.results[?(@.name=="base-commit")].value'; echo ---; kubectl -n vf-ci logs -l "tekton.dev/pipelineRun=$run" --all-containers --prefix --tail=40 --max-log-requests=12
```

A check passed when its STATUS is `Succeeded` and failed when it is `Failed`.
The pipeline's own STATUS is `Failed` when any one check failed, as a GitHub
workflow run is.

## Comparing with GitHub

Both executors give the same verdict when they see the same inputs. Three
inputs change over time, so compare runs started close together:

- **The base tip.** `test-erosion` counts tests at the tip of `main` as fetched,
  not at the merge base. If `main` gained tests between the two runs, the
  verdicts can differ. The `BASE` column shows the tip Tekton used.
- **The advisory databases.** `just check` runs cargo-deny and cargo-audit
  against the latest RustSec data. A new advisory turns every commit red on
  both executors from the moment it is published.
- **`just`.** The workflow installs the latest release; the Task pins 1.58.0.
  If `just` publishes a new release, update the pin and digests in
  `tasks/rust-check.yaml`.

## Clean up

The manual runs stay until deleted. Deleting a PipelineRun deletes its
TaskRuns and pods:

```sh
kubectl -n vf-ci delete pipelinerun -l tekton.dev/pipeline=vf-lane-gate; kubectl -n vf-ci delete pipelinerun -l tekton.dev/pipeline=vf-rust-check
```

## Roll back

Clean up as above, then remove the five definitions by name. Namespace
`vf-ci` and everything from `triggers/` stay:

```sh
kubectl -n vf-ci delete pipeline vf-lane-gate vf-rust-check; kubectl -n vf-ci delete task vf-lane-gate vf-lane-gate-selftest vf-rust-check
```

## Not here yet

- `check (amd64)`: VFL-716, see "Architectures".
- Starting these pipelines from the webhook. `triggers/trigger-template.yaml`
  still starts `vf-ci-noop`. Before it starts a pipeline that checks out
  pull-request code, a policy for pull requests from forks is needed. The
  pipelines here hold no credential and mount no token, which is one part of
  such a policy.
- Pruning of finished runs, and a LimitRange or ResourceQuota on `vf-ci`.
- Reporting each check to the PR commit as a status with the same context
  name, and switching the required contexts on `main` (VFL-126).
