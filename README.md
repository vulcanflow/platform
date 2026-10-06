# vulcanflow / platform

The Rust backend of VulcanFlow: the control plane, the domain core, the operator
and the worker-side binaries. The TypeScript console lives in its own
repository; this one is backend only.

Everything here builds and tests on a developer machine. No step requires
Kubernetes, Harbor, Argo CD or any hosted service.

## Architecture agnostic

**arm64 and amd64 are equal first-class targets.** No code, dependency, feature
flag, build recipe or workflow in this repository may assume one of them, and
that holds regardless of which architecture the developer who wrote it happens
to be sitting on. In practice:

- `rust-toolchain.toml` installs `aarch64-unknown-linux-gnu` *and*
  `x86_64-unknown-linux-gnu` explicitly, so the same checkout gives both
  developers the same target set. It does not rely on the host target coming
  along for free, because that is exactly how one architecture quietly stops
  being checked.
- `deny.toml` evaluates the dependency graph for both GNU targets, so a crate
  that only resolves on one of them is a `cargo deny` failure, not a release
  surprise.
- `just check-cross` builds the workspace for whichever of the two is *not*
  this machine, derived at run time. Run it before handing work over.
- The only `cfg(target_arch)` in the tree is `wasm32` in `vf-graph`, which
  selects the browser bindings. That is a platform distinction, not a CPU
  one, and it is the only kind allowed.
- Release images are multi-architecture. Image building itself is deferred
  (see *Deferred*), but nothing in this repository may make it harder.

## Crate inventory (architecture §A1.3)

Exactly these sixteen crates, no more and no fewer. There is no `vf-store`.

| Crate | Kind | Responsibility |
| --- | --- | --- |
| `vf-core` | lib | I/O-free domain core: identities, scope, state machines, policy, problems, ports. |
| `vf-db` | lib | Postgres schemas, migrations, tenant/control transactions, repositories, outbox, infrastructure adapters. |
| `vf-graph` | lib | Pipeline graph DSL, type lattice, validation and JSON schema; native and wasm32. |
| `vf-translator` | lib | Validated graph plus run scope to work-unit plans, SCB objects and per-node argv. |
| `vf-authz` | lib | Track A challenges, authorization basis lifecycle and the start-barrier decision. |
| `vf-meter` | lib | Atomic reserve/settle/release accounting, billing-period usage and target admission. |
| `vf-remediation` | lib | Guidance resolution, verification plan derivation and outcome interpretation. |
| `vf-api` | lib + bin | Control-plane REST and SSE surface, OpenAPI 3.1, auth, dispatcher and outbox worker. |
| `vf-operator` | lib + bin | ScanFlow reconcile loop, scan runtimes and tenant provisioning. |
| `vf-admission` | lib + bin | Fail-closed validating admission handler backed by the start-barrier decision. |
| `vf-ingest` | lib + bin | Signed notification receiver, bounded artifact parse, settlement and reconciliation sweep. |
| `vf-hook-notify` | bin | secureCodeBox completion hook: builds, signs and posts the ingest notification. |
| `vf-scanner-adapter` | bin | Runs inside scanner images: materializes inputs, builds argv, classifies outcomes. |
| `vf-report` | lib + bin | Report assembly, immutable input snapshots, templating and delivery. |
| `vf-abuse` | bin | Phase 4 abuse handling. Present so the §A1.3 inventory is complete; no task yet. |
| `vf-testkit` | lib | Dev-dependency-only test harness: Postgres bootstrap, fixtures, deterministic clock and ids. |

`vf-api` also carries a second, non-deployed binary, `vf-openapi`, which dumps
the generated OpenAPI document so the 3.1 output can be diffed.

### Dependency direction (§A1.4)

`vf-core` is I/O free: it must never reach `tokio`, `sqlx`, `reqwest` or `kube`.
`vf-graph` must stay portable to the browser, so it carries no I/O crate on
`wasm32-unknown-unknown` either.

Most of §A1.4 is enforced mechanically by the `bans.wrappers` entries in
`deny.toml`, which list exhaustively who may depend on each internal crate.
These two rules are the exception: they cannot be written as bans, because the
legitimate direct dependents of `tokio`, `sqlx`, `reqwest` and `kube` are
third-party crates whose set changes on every bump. They are enforced by
`just graph-rules` (`cargo tree`, per target) and by the workspace test pack
T9 instead. Dropping `graph-rules` from `just gate` drops the only
pre-push check of them.

## Delivery lanes

Production code and tests are separate deliverables written by separate agents,
and a pull request may not mix them. `ci/lane-gate.sh` enforces this by
partitioning the diff on paths rather than on authors, because every agent
reaches GitHub through one shared identity.

| Lane | Paths | Written by |
| --- | --- | --- |
| PROD | `crates/*/src/**`, `crates/*/build.rs`, `crates/*/Cargo.toml`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml` | the coding agents |
| TEST | `tests/**`, `crates/*/tests/**`, `fuzz/**`, `conformance/**`, `**/testdata/**`, `**/golden/**`, `*.golden`, `*.snap` | the test authors |
| GATE | `ci/lane-gate.sh`, `ci/lane-gate-test.sh`, `.github/workflows/lane-gate.yml` | the gate owner |
| NEUTRAL | everything else | any lane |

The gate also refuses `#[cfg(test)]` inside production source, and refuses
diffs that ignore, delete or thin out existing tests. `just lane-gate` runs all
three checks on this working tree against `origin/main`;
`just lane-gate-selftest` is a different thing — it replays fixture diffs
through the gate in a throwaway repository to check the gate itself. `just gate`
runs both.

Nothing is pushed until the matching test pack passes under the test runner,
both reviewers have reviewed the same code/test pair, and the architect has
approved that exact revision pair.

## Working on it

On a machine that already has the pinned toolchain and a C compiler, go
straight to the recipes. On one that has neither — the agent runners are the
case this exists for — bootstrap first:

```
eval "$(just bootstrap)"   # or: eval "$(ci/bootstrap-toolchain.sh)"
```

That is needed because nothing in this workspace compiles without a C
toolchain: `serde_derive` and `thiserror-impl` are proc macros, so `cargo
check` builds their build scripts for the host and fails on `linker 'cc' not
found` before reaching any crate of ours. The runners have `rustup` available
but no `cc`, no `ld` and no libc headers, and at uid 65532 cannot `apt-get
install` them.

`ci/bootstrap-toolchain.sh` therefore *unpacks* rather than installs: it
fetches the packages pinned in `ci/toolchain/debs-<arch>.lock`, verifies each
SHA256, and extracts them with `dpkg-deb -x` into a sysroot beside the
checkout, then puts `cc`/`ar`/`ld` shims and the pinned Rust toolchain on
`PATH`. It needs no privilege beyond writing one directory and changes nothing
outside it. The prefix is shared by every task on the workspace and is reused,
so only the first run pays for the download; `--env-only` prints the
environment without touching the network. Override the location with
`VF_TOOLCHAIN_DIR`.

The lock is generated by `ci/toolchain/resolve-debs.py`, not hand-edited, and
`libc6`/`libc6-dev` are pinned to the runner image's own glibc — see the header
of each script for why both of those matter.

```
just            # list recipes
just check      # fmt, clippy -D warnings, cargo deny, cargo audit
just build      # cargo build --workspace --all-targets
just test-unit  # unit tests, no services
just wasm       # vf-graph on wasm32-unknown-unknown
just check-cross# the workspace on the other CPU architecture
just lane-gate  # the delivery-lane gate on this tree, vs origin/main
just gate       # lane gate and its self-test, then all of the above
```

`just test-integration` additionally needs the compose stand-ins from
`just dev-up` (Postgres 17, PgBouncer, RustFS, Valkey).

## Pins

The toolchain is pinned in `rust-toolchain.toml` and every dependency is pinned
to an exact `=` version in `[workspace.dependencies]`, with `Cargo.lock`
committed. Crates do not declare versions of their own; they take
`{ workspace = true }`. A bump is a reviewed change to the architecture §A5
table first and to this workspace second.

There are no open deviations from the §A5 table: every row this workspace
carries is the ratified one (§A5 revision 4). Where a pin's feature set is not
self-explanatory — the four TLS-bearing rows, and `testcontainers` — the reason
it reads the way it does is recorded in a comment next to the pin, because the
one-line edits that would undo it are not obviously wrong on sight.

## Deferred

These are deployment concerns and do not block local work: multi-architecture
release images, Harbor signing, SBOM publication.
