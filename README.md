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

`vf-core` is I/O free: it must never reach `tokio`, `sqlx`, `reqwest` or `kube`,
and `cargo tree -p vf-core` is the check. `vf-graph` must stay portable to the
browser, so it carries no I/O crate on `wasm32-unknown-unknown` either. Both
rules are enforced mechanically by the `bans.wrappers` entries in `deny.toml`,
not by convention.

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
diffs that ignore, delete or thin out existing tests. Run it with
`just lane-gate`.

Nothing is pushed until the matching test pack passes under the test runner,
both reviewers have reviewed the same code/test pair, and the architect has
approved that exact revision pair.

## Working on it

```
just            # list recipes
just check      # fmt, clippy -D warnings, cargo deny, cargo audit
just build      # cargo build --workspace --all-targets
just test-unit  # unit tests, no services
just wasm       # vf-graph on wasm32-unknown-unknown
just check-cross# the workspace on the other CPU architecture
just gate       # lane gate, then all of the above
```

`just test-integration` additionally needs the compose stand-ins from
`just dev-up` (Postgres 17, PgBouncer, RustFS, Valkey).

## Pins

The toolchain is pinned in `rust-toolchain.toml` and every dependency is pinned
to an exact `=` version in `[workspace.dependencies]`, with `Cargo.lock`
committed. Crates do not declare versions of their own; they take
`{ workspace = true }`. A bump is a reviewed change to the architecture §A5
table first and to this workspace second.

Deviations from the §A5 table are recorded inline in `Cargo.toml` next to the
pin they apply to, each with its reason, and on the task that introduced them.

## Deferred

These are deployment concerns and do not block local work: multi-architecture
release images, Harbor signing, SBOM publication.
