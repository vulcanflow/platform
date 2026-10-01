# VulcanFlow — Rust workspace

The single Rust build unit containing all VulcanFlow crates, with one pinned
toolchain and one committed lockfile (TDD §2.5.2, ADR-0001).

One workspace rather than one repository per service is a decision, not an
accident: six independent Rust bootstraps would produce six lockfiles and six
toolchains, and would reintroduce `vf-core` version skew across exactly the
crates that hold authorization scope, allowance accounting and finding state.

## Layout

| Crate | Kind | What it holds |
|---|---|---|
| `vf-core` | library, **no I/O** | Authorization scope matching, allowance reservation and settlement, observation state, role policy |
| `vf-authz` | library, **no I/O** | The authorization decision and the role × action policy |
| `vf-graph` | library, **no I/O**, native + wasm | The §7.3 flow-graph validation rule set, shared with the browser builder |
| `vf-translator` | library, **no I/O** | Flow graph → secureCodeBox `Scan` / `CascadingRule` |
| `vf-db` | library | `TenantTx` and the RLS-backed tenant isolation primitives |
| `vf-store` | library | The single S3 client constructor; non-AWS configuration is mandatory |
| `vf-api` | binary | REST, OpenAPI 3.1, SSE, webhook receiver, dispatcher |
| `vf-meter` | binary | Allowance accounting, usage ledger, billing sync |
| `vf-ingest` | binary | Findings artifact ingest |
| `vf-operator` | binary | Reconciles the `Tenant` and `ScanFlow` CRDs |
| `vf-admission` | binary | Admission webhooks, fail-closed |
| `vf-hook-notify` | binary | secureCodeBox completion hook |
| `vf-report` | binary | Report assembly and rendering |
| `vf-abuse` | binary | KYC orchestration, anomaly detection |

The four crates marked **no I/O** take values and return values: no database,
no clock, no network, no filesystem. Time and identifiers are parameters. That
is what makes them property-testable and fuzzable in isolation, and it is what
makes the `vf-graph` WebAssembly build possible without a host interface.
`ci/crate-boundaries.sh` asserts it on every build, because the property is lost
silently the first time a convenience dependency pulls a runtime in
transitively.

## Rules that are enforced rather than remembered

| Rule | Where it is enforced |
|---|---|
| `#![forbid(unsafe_code)]` in every crate root | the attribute itself, the workspace lint table, and `ci/forbid-unsafe.sh` |
| No `unwrap`/`expect`/`panic` | `[workspace.lints.clippy]`, denied |
| No floating point or decimal types in the allowance path | `clippy::float_arithmetic` denied; `rust_decimal` and `bigdecimal` banned in `deny.toml` |
| Checked integer arithmetic | `clippy::arithmetic_side_effects` denied, so `checked_*` is the only way to write it |
| Deterministic iteration order | `clippy::disallowed_types` denied for `HashMap`/`HashSet`; see `clippy.toml` |
| One rustls crypto provider (`aws-lc-rs`), no `ring` | `deny.toml` ban plus `ci/tls-provider-assertions.sh` |
| `jiff`, not `chrono` | `deny.toml` ban plus the same script |
| Pinned versions in exactly one place | `[workspace.dependencies]`; members inherit with `workspace = true` |

Every dependency version is an ADR-0002 §3 pin. Changing one is an ADR
amendment, not a manifest edit.

## Toolchain

`rust-toolchain.toml` pins the channel; `rustup show` installs it. `Cargo.lock`
is committed, and every CI invocation passes `--locked`, which fails rather than
updating it.

```
rustup show                                 # installs the pinned toolchain
cargo check --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

`vf-graph` additionally builds for the browser:

```
rustup target add wasm32-unknown-unknown
cargo build --locked --release -p vf-graph --target wasm32-unknown-unknown
```

## CI

| Workflow | Job | Covers |
|---|---|---|
| `rust-build.yml` | `fmt-clippy` | `cargo fmt`, `check`, `clippy -D warnings`, `cargo doc`, crate boundaries |
| `rust-build.yml` | `graph-native-and-wasm` | `vf-graph` for both targets — what §25 `graph/native-wasm-parity` runs against |
| `rust-supply-chain.yml` | `supply-chain` | lockfile currency, `cargo deny`, `cargo audit`, the ADR-0002 §3.7.9 per-binary TLS/crypto assertions, `#![forbid(unsafe_code)]`, CycloneDX SBOM |
| `rust-supply-chain.yml` | `reproducible-arm64` | two release builds, byte-identical artifacts |
| `rust-test.yml` | `test` | invokes the suites |

Two gates are allowlists rather than searches for known-bad names, and that is
deliberate: `ci/crate-boundaries.sh` diffs each pure crate's complete transitive
set against `ci/pure-crate-dependencies.txt`, and `ci/tls-provider-assertions.sh`
asserts the one permitted path for a second `digest` rather than waiving the
version. A denylist only catches what someone already thought of.

## Tests

There are none in this tree yet, and no coding agent adds one. Scribe writes the
property and fuzz tests, Ledger writes the integration suites, and Crucible runs
them. A failing test is a defect in the code; it is never weakened, skipped or
`#[ignore]`d to make a suite green. TDD §23.2 is explicit that a named §25 test
identifier existing is not a claim that it passes.
