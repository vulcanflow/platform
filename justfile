# vulcanflow / platform — developer entry points.
#
# Every recipe runs on a developer machine with nothing but the pinned
# toolchain. Nothing here reaches Kubernetes, Harbor, Argo CD or any hosted
# service. No recipe hard-codes a CPU architecture: the project is
# architecture agnostic and must build identically on arm64 and amd64.

set shell := ["bash", "-euo", "pipefail", "-c"]

# Every recipe below is a non-interactive bash, and bash sources $BASH_ENV on
# startup. The agent runners point BASH_ENV at a generated .bashrc that assigns
# PATH *absolutely*, which deletes the bootstrapped toolchain from each recipe's
# PATH — `just check` then fails with `cargo: command not found` even though
# cargo resolves fine in the caller's own shell. Clearing it for recipes makes
# that independent of whether the caller remembered to clear it as well.
# A no-op on a developer machine, where BASH_ENV is not set in the first place.
export BASH_ENV := ''

# The compose front end. `docker compose` by default; set VF_COMPOSE to
# "podman compose" or "docker-compose" if that is what you have. A plain
# assignment rather than a backtick detection, because just evaluates
# top-level backticks on every invocation — including `just --list` — and the
# list of recipes must work on a machine that has none of this installed.
compose := env_var_or_default("VF_COMPOSE", "docker compose")

# How long `just dev-up` waits for the default services. The F2 acceptance
# criterion is two minutes on a clean machine, which on a cold pull is mostly
# download time.
dev_up_timeout := env_var_or_default("VF_DEV_UP_TIMEOUT", "120")

default:
    @just --list

# Toolchain bootstrap for a machine that has no C compiler and no Rust — the
# agent runners. Prints the environment to stdout and its progress to stderr,
# so the whole thing is `eval "$(just bootstrap)"`. Idempotent and cached: a
# second run re-prints the environment without downloading anything. Not a
# dependency of any other recipe, because on a normal developer machine the
# toolchain is already there and this must not run behind their back.
bootstrap:
    @ci/bootstrap-toolchain.sh

# ---------------------------------------------------------------------------
# The local harness — architecture §A6.2
#
# The walking loop on one machine:
#
#     just dev-up && just db-migrate && just run-local
#
# Every recipe here loads `.env` first, so the compose stand-ins and the Rust
# binaries read one set of values. `.env` is gitignored; `.env.example` is the
# template and documents every variable.
# ---------------------------------------------------------------------------

# Brings up the default stand-ins — Postgres 17 with pgvector, PgBouncer in
# transaction pooling, RustFS, Valkey — and does not return until they answer.
#
# Optional profiles go through COMPOSE_PROFILES, which compose reads itself, so
# one spelling works for every recipe here rather than only this one:
#
#     COMPOSE_PROFILES=minio just dev-up      # second S3 implementation
#     COMPOSE_PROFILES=keycloak just dev-up   # the real IdP (§A6.4), optional
#
# Needs Docker Compose v2.17 or newer, for `up --wait`.
dev-up:
    #!/usr/bin/env bash
    set -euo pipefail

    if [ ! -f .env ]; then
      echo "==> no .env yet; copying .env.example"
      cp .env.example .env
    fi
    set -a; . ./.env; set +a

    if ! {{ compose }} version >/dev/null 2>&1; then
      echo "error: '{{ compose }}' does not work here." >&2
      echo "       Install Docker Compose v2, or set VF_COMPOSE — for example" >&2
      echo "       VF_COMPOSE='podman compose' just dev-up" >&2
      exit 1
    fi

    echo "==> starting the default services"
    # --wait blocks on every healthcheck declared in docker-compose.yml.
    # pgbouncer and rustfs declare none, for the reason given in that file, so
    # they are waited on below from the host, where bash is guaranteed.
    {{ compose }} up -d --wait --wait-timeout {{ dev_up_timeout }}

    wait_tcp() {
      local name="$1" host="$2" port="$3"
      local deadline=$(( SECONDS + {{ dev_up_timeout }} ))
      printf '==> waiting for %s on %s:%s' "$name" "$host" "$port"
      until (exec 3<>"/dev/tcp/${host}/${port}") 2>/dev/null; do
        if [ "$SECONDS" -ge "$deadline" ]; then
          printf ' timed out after %ss\n' '{{ dev_up_timeout }}' >&2
          echo "    see: {{ compose }} logs ${name}" >&2
          exit 1
        fi
        printf '.'
        sleep 1
      done
      printf ' ok\n'
    }

    wait_tcp pgbouncer 127.0.0.1 "${VF_PGBOUNCER_PORT:-6432}"
    wait_tcp rustfs 127.0.0.1 "${VF_S3_PORT:-9000}"

    echo
    {{ compose }} ps
    echo "==> ready. next: just db-migrate && just run-local"

# Stops the stand-ins. Data survives in the named volumes.
#
#     just dev-down --volumes     discard the data too, which is also how you
#                                 recover from a half-initialised Postgres
dev-down *args:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -f .env ]; then set -a; . ./.env; set +a; fi
    {{ compose }} down {{ args }}

# Applies the `vf-db` migrations to the database in DATABASE_URL.
#
# Straight at Postgres, never through PgBouncer: DDL in transaction pooling
# mode is a good way to deadlock against a connection you cannot see, so
# `.env.example` points DATABASE_URL at the direct port.
db-migrate:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -f .env ]; then set -a; . ./.env; set +a; fi

    if ! command -v sqlx >/dev/null 2>&1; then
      echo "error: sqlx-cli is not on PATH." >&2
      echo "       cargo install sqlx-cli --no-default-features --features rustls,postgres" >&2
      exit 1
    fi

    src=crates/vf-db/migrations
    if [ ! -d "$src" ] || [ -z "$(find "$src" -maxdepth 1 -name '*.sql' -print -quit)" ]; then
      echo "==> no migrations: ${src} is absent or holds no .sql file."
      echo "    Task C1 owns the control schema and the tenant template, so until it"
      echo "    lands the migration set is empty and applying it is a no-op."
      exit 0
    fi

    echo "==> applying ${src}"
    sqlx migrate run --source "$src"

# Rebuilds the sqlx template database and regenerates the offline query data
# in `.sqlx/`.
#
# `.sqlx/` is what lets the workspace compile its compile-time-checked queries
# without a database — in CI, in a release build, on a machine with nothing
# running — so it is committed, and it has to be reproducible: the same
# migrations and the same queries must give the same bytes. Two things make
# that true here. `.sqlx/` is deleted before it is regenerated, so a query
# that was removed cannot leave a stale file behind; and the generation runs
# against a database built from the migrations alone, never against whatever
# state a developer's dev database has drifted into.
#
# That template database is separate from the dev one and is dropped and
# recreated every time. Your dev data is not touched.
db-template:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -f .env ]; then set -a; . ./.env; set +a; fi

    if ! command -v sqlx >/dev/null 2>&1; then
      echo "error: sqlx-cli is not on PATH." >&2
      echo "       cargo install sqlx-cli --no-default-features --features rustls,postgres" >&2
      exit 1
    fi

    direct="${VF_DATABASE_URL_DIRECT:?set VF_DATABASE_URL_DIRECT in .env}"
    template_db="${VF_PG_DATABASE:-vulcanflow}_sqlx_template"

    # Swap the database name, keeping any query string the URL carries.
    base="${direct%%\?*}"
    query=""
    if [ "$base" != "$direct" ]; then query="?${direct#*\?}"; fi
    template_url="${base%/*}/${template_db}${query}"

    echo "==> recreating ${template_db}"
    # A drop of a database that is not there is the one expected failure.
    sqlx database drop -y --database-url "$template_url" >/dev/null 2>&1 || true
    sqlx database create --database-url "$template_url"

    src=crates/vf-db/migrations
    if [ -d "$src" ] && [ -n "$(find "$src" -maxdepth 1 -name '*.sql' -print -quit)" ]; then
      echo "==> applying ${src}"
      sqlx migrate run --source "$src" --database-url "$template_url"
    else
      echo "==> no migrations yet (task C1); the template is an empty database"
    fi

    echo "==> regenerating .sqlx/"
    rm -rf .sqlx
    DATABASE_URL="$template_url" cargo sqlx prepare --workspace -- --all-targets

    echo "==> done. 'git diff --stat .sqlx' should be empty unless a query changed."

# Runs the control plane locally: vf-api, vf-operator with the fake scan
# runtime, and vf-ingest, together, in the foreground.
#
# `--runtime fake` is passed explicitly rather than left to VF_SCAN_RUNTIME,
# because this recipe is the one place where "no Kubernetes is involved" has to
# be true by construction and not by configuration.
#
# If any of the three exits, the other two are stopped. A control plane with
# the ingest endpoint missing is not a running system, and discovering that
# from a silent 404 half an hour later is worse than stopping.
run-local:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -f .env ]; then set -a; . ./.env; set +a; fi

    echo "==> building"
    cargo build -p vf-api -p vf-operator -p vf-ingest --bins

    bin="${CARGO_TARGET_DIR:-target}/debug"
    pids=()
    cleanup() {
      trap - INT TERM EXIT
      echo
      echo "==> stopping"
      if [ ${#pids[@]} -gt 0 ]; then kill "${pids[@]}" 2>/dev/null || true; fi
      wait 2>/dev/null || true
    }
    trap cleanup INT TERM EXIT

    "$bin/vf-api" & pids+=("$!")
    "$bin/vf-operator" --runtime fake & pids+=("$!")
    "$bin/vf-ingest" & pids+=("$!")

    echo "==> vf-api, vf-operator (fake runtime) and vf-ingest are up. Ctrl-C to stop."
    wait -n

# Reserved for later infrastructure work.
#
# §A6.3 is explicit about this: "a kind profile is reserved in the harness
# (`just kind-up`) but no task in this document is verified on it". The real
# admission registration, CRD watch, NetworkPolicy and pool paths are
# deployment work that the owner has not authorised and that no acceptance
# criterion depends on. The recipe exists so the name is taken by something
# honest rather than by a half-built cluster.
kind-up:
    @echo "reserved for later infrastructure work"

# Full local gate: formatting, lints, supply chain. Matches the definition of
# done for every coding task.
#
# `deny` and `audit` are two deliberate advisory passes, not a duplicate (§A5,
# ruling on VFL-98): `deny` applies the whole reviewed `deny.toml` policy to the
# graph resolved with `all-features` and filtered to the three §A5 targets,
# while `audit` denies every informational class across the whole `Cargo.lock`,
# unfiltered by target or feature. Dropping either loses coverage.
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
    #!/usr/bin/env bash
    set -euo pipefail
    # gix — the git implementation compiled into cargo-audit, not the git CLI —
    # refuses to load any git configuration when GIT_CONFIG_COUNT is set but
    # empty, which is how the agent runners export it. It surfaces as
    # `failed to prepare clone` against the advisory database, so it reads like
    # a network or disk fault rather than an environment one. The bootstrap's
    # env clears it, but this recipe is also run on its own by Test Runner.
    [ -n "${GIT_CONFIG_COUNT:-}" ] || unset GIT_CONFIG_COUNT
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
