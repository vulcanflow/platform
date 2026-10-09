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

The C toolchain is kept per lock, under `c-toolchain/<lock digest>/` in the
prefix, and a published tree is never deleted. A branch that changes the lock
therefore builds its own tree beside the existing one rather than replacing a
compiler another run is using. It also means `--env-only` fails on such a branch
until it has been bootstrapped once, instead of handing it the old lock's
compiler.

The lock is generated by `ci/toolchain/resolve-debs.py`, not hand-edited, and
`libc6`/`libc6-dev` are pinned to the runner image's own glibc — see the header
of each script for why both of those matter.

Use `eval`, not `. env.sh` or a hand-rolled `export PATH=…`. The bootstrap's
environment ends with `unset BASH_ENV`, and on an agent runner that line is
load-bearing: the harness points `BASH_ENV` at a generated `.bashrc` that
assigns `PATH` absolutely, so every non-interactive bash — including each
`just` recipe — starts with the toolchain stripped back off its `PATH`. Set
`PATH` by hand and `cargo` resolves in your shell but not inside `just check`:

```
cargo fmt --all -- --check
bash: line 1: cargo: command not found
```

The `justfile` clears `BASH_ENV` for its own recipes too, and `just audit`
clears an empty `GIT_CONFIG_COUNT`, so `just` works either way; the `eval` is
what fixes the plain `bash -c` and `cargo test` cases. If a runner reports "no
rustc here", check this before concluding the toolchain is missing.

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

## The local harness (architecture §A6.2)

Everything the platform talks to runs in `docker-compose.yml` on one machine.
No Kubernetes, no Aether, no cloud account.

```
cp .env.example .env    # the template, and the documentation for every variable
just dev-up             # postgres + pgvector, pgbouncer, rustfs, valkey
just db-migrate         # apply the vf-db migrations
just run-local          # vf-api, vf-operator --runtime fake, vf-ingest
just dev-down           # stop (add --volumes to discard the data)
```

`just dev-up` returns only once the default services answer, so the next
recipe can assume them. `just run-local` stops the other two services when any
one exits. Until tasks A1, O1 and I1 land, all three are empty entry points,
so it stops straight after the build. If your front end is not
`docker compose`, set `VF_COMPOSE` in your shell environment, not in `.env` —
for example `VF_COMPOSE='podman compose' just dev-up`.

Every image is pinned by tag and digest in `docker-compose.yml`, the one place
the pins live. A `VF_*_IMAGE` variable in `.env` overrides one on your machine
only, to try a release before proposing it as the pin.

Optional profiles, through compose's own `COMPOSE_PROFILES`:

| Profile | What for |
|---|---|
| `minio` | the second S3 implementation, so the conformance suite runs against two. Chainguard's build of MinIO, since neither of MinIO's own images can be pulled anonymously; `docker-compose.yml` records the release behind the digest and what to do if it stops resolving |
| `keycloak` | the real identity provider and its PKCE flow (§A6.4). Documented, required by no recipe and no task; tests use the dev issuer. It has no healthcheck, so start it as `docker-compose.yml` shows rather than through `just dev-up` |
| `mailpit` | SMTP sink for the M4 mail work |

`just db-template` rebuilds a throwaway template database from the migrations
and regenerates the committed `.sqlx/` offline query data from it, so a build
with no database reachable still type-checks every query. It does not touch
your dev database.

`just kind-up` prints `reserved for later infrastructure work` and exits. §A6.3
reserves the name; no task is verified on a cluster.

## Pins

The toolchain is pinned in `rust-toolchain.toml` and every dependency is pinned
to an exact `=` version in `[workspace.dependencies]`, with `Cargo.lock`
committed. Crates do not declare versions of their own; they take
`{ workspace = true }`. A bump is a reviewed change to the architecture §A5
table first and to this workspace second.

There are no open deviations from the §A5 table: every row this workspace
carries is the ratified one (§A5 revision 5). Where a pin's feature set is not
self-explanatory — the four TLS-bearing rows, and `testcontainers` — the reason
it reads the way it does is recorded in a comment next to the pin, because the
one-line edits that would undo it are not obviously wrong on sight.

Every `uses:` in `.github/workflows/**` carries a full 40-character commit SHA,
with the tag it resolved to as a trailing comment. A tag is mutable, so a
re-point would silently change what CI executes. Resolve a new one with
`git ls-remote <repo> 'refs/tags/<tag>^{}'`, falling back to `refs/tags/<tag>`
when that is empty because the tag is lightweight, and keep the comment in step
with the SHA.

## Deferred

These are deployment concerns and do not block local work: multi-architecture
release images, Harbor signing, SBOM publication.
