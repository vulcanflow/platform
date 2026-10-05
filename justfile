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

deny:
    cargo deny --all-features check

audit:
    cargo audit --deny warnings

build:
    cargo build --workspace --all-targets

# Unit tests only — the crate-local pack. Needs no services.
test-unit:
    cargo test --workspace --lib --bins

# Integration tests — needs the compose stand-ins from `just dev-up`
# (Postgres 17 + PgBouncer + RustFS + Valkey).
test-integration:
    cargo test --workspace --test '*'

# The pre-push gate. Lane partition first, because a mixed pull request is
# refused before anything else is worth running.
gate: lane-gate check build test-unit wasm graph-rules check-cross

lane-gate:
    ci/lane-gate-test.sh ci/lane-gate.sh

# vf-graph must stay portable to the browser: pure evaluation, no I/O crate.
wasm:
    cargo build -p vf-graph --target wasm32-unknown-unknown

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
