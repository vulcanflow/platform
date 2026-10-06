# vulcanflow / platform — developer entry points.
#
# Every recipe runs on a developer machine with nothing but the pinned
# toolchain. Nothing here reaches Kubernetes, Harbor, Argo CD or any hosted
# service. No recipe hard-codes a CPU architecture: the project is
# architecture agnostic and must build identically on arm64 and amd64.

set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

# Full local gate: formatting, lints, supply chain. Matches the definition of
# done for every coding task.
check: fmt-check clippy deny audit

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

clippy:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# No `--all-features` here: `deny.toml`'s `[graph] all-features` is the single
# place that choice is made, and a flag would override the committed value.
deny:
    cargo deny check

audit:
    cargo audit --deny warnings

build:
    cargo build --workspace --all-targets

# Unit tests only — the crate-local pack. Needs no services.
test-unit:
    cargo test --workspace --lib --bins

# Integration tests — needs the compose stand-ins from `just dev-up`
# (Postgres 17 + PgBouncer + RustFS + Valkey). `--features integration` is the
# §A6.2 switch; every crate declares it, and without it the suite compiles with
# those tests cut out and still reports success.
test-integration:
    cargo test --workspace --test '*' --features integration

# The pre-push gate. The lane gate runs first, because a diff that mixes
# production and test files is refused before anything else is worth running.
gate: lane-gate lane-gate-selftest check build test-unit wasm graph-rules check-cross

# The lane gate itself (§A6.2): diff partition, test erosion and #[cfg(test)]
# in production source, against origin/main.
lane-gate:
    ci/lane-gate.sh all

# The lane gate's own self-test: it replays fixture diffs through the gate in a
# throwaway repository and asserts its exit codes. That checks the gate, not
# this working tree, which is why it is a recipe of its own and not `lane-gate`.
lane-gate-selftest:
    ci/lane-gate-test.sh ci/lane-gate.sh

# vf-graph must stay portable to the browser: pure evaluation, no I/O crate.
# Debug, because this is the portability check; `wasm-release` is the one the
# §A5 size budget is measured on.
wasm:
    cargo build -p vf-graph --target wasm32-unknown-unknown

# The browser artefact, built with the size-optimised profile that carries §A5's
# 300 KiB gzipped budget. Not part of `gate`: the budget belongs to the task
# that owns the SPA bundle.
wasm-release:
    cargo build -p vf-graph --target wasm32-unknown-unknown --profile wasm-release

# Architecture agnosticism check: build the workspace for the Linux GNU target
# that is not this machine's. Catches an architecture-specific dependency or
# cfg the day it lands rather than at release time.
#
# The host is detected inside the recipe, not in a top-level `just` variable:
# just evaluates those on every invocation, so a backtick here would make
# `just --list` fail on a machine without the toolchain on PATH.
check-cross:
    #!/usr/bin/env bash
    set -euo pipefail
    host="$(rustc -vV | sed -n 's/^host: //p')"
    case "$host" in
      aarch64-*) other=x86_64-unknown-linux-gnu ;;
      x86_64-*)  other=aarch64-unknown-linux-gnu ;;
      *) echo "unsupported host $host; expected aarch64 or x86_64 Linux" >&2; exit 1 ;;
    esac
    echo "host $host -> cross-checking $other"
    cargo check --workspace --target "$other"

# The §A1.4 crate-graph rules deny.toml cannot express as bans: vf-core and
# vf-graph stay I/O free, and no binary crate depends on another binary crate.
# See the header of the script for why each one cannot be a ban.
graph-rules:
    ci/graph-rules.sh

# Emits the OpenAPI document so §27-17 evidence can be diffed.
openapi:
    cargo run -p vf-api --bin vf-openapi
