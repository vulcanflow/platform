#!/usr/bin/env bash
# Asserts the "no I/O in the core" boundary (TDD §2.5.2).
#
# `vf-core`, `vf-authz`, `vf-graph` and `vf-translator` hold the safety-critical
# state machines: authorization scope, allowance accounting, observation state,
# role policy, graph validation. They take values and return values.
#
# The rule is a CI assertion rather than a convention because what it buys —
# exhaustive property testing and fuzzing in isolation, and a `vf-graph` build
# for `wasm32-unknown-unknown` with no host interface — is lost the moment one
# convenience dependency pulls a runtime in transitively, and nothing else in
# the build would notice. VUL-24, VUL-25, VUL-26 and VUL-29 state this
# assertion as their acceptance criterion.
#
# ------------------------------------------------------------------------------
# ALLOWLIST, NOT DENYLIST, and that is the whole design.
#
# An earlier revision grepped each crate's resolved graph for sixteen named I/O
# crates. That is a gate that only catches dependencies someone already thought
# of: it passes clean on `ureq`, `smol`, `async-std`, `async-io`, or `getrandom`
# with an OS backend — and that last one breaks the `wasm32-unknown-unknown`
# build this gate exists to protect. A boundary has to hold against the
# dependency nobody has imagined yet, so the mechanism is a diff against a
# recorded set, not a search for known-bad names.
#
# `ci/pure-crate-dependencies.txt` records the complete expected transitive set
# per pure crate. This script resolves each crate and diffs. Any addition fails;
# so does any removal, because a stale record is a record nobody is reading.
# Updating it is `ci/crate-boundaries.sh --record`, which produces a diff in a
# committed file that a reviewer has to look at — which is the point.
#
# Names, not name@version: this gate is about what a crate *reaches*, and
# `--locked` plus cargo-deny and cargo-audit are what police versions. Recording
# versions here would turn every patch bump into a red boundary gate, and a gate
# that is red for reasons nobody can act on stops being read.
#
# `--target all`: a dependency that appears only under some `cfg(target_*)` is
# still a boundary breach, and resolving every target makes the record
# independent of which runner evaluates it.
#
# `--edges normal`: dev-dependencies are excluded on purpose — the test authors'
# harnesses may use a runtime, the shipped library may not. Proc-macro edges are
# deliberately *included*, even though proc macros run on the build host and do
# not ship, because they are few, they are stable under a pinned lockfile, and a
# new one appearing in a pure crate is worth one line of review.
# ------------------------------------------------------------------------------
#
# The reason table below is documentation, not the mechanism. The diff decides
# the exit code; these names only make a failure message say *why* a particular
# arrival is disqualifying, for the common cases. Adding a crate here does not
# make the gate stronger, and leaving one out does not make it weaker.
#
#   tokio, mio, socket2        an async runtime and its syscall layer
#   sqlx                       a database client
#   reqwest, hyper, hyper-util an HTTP client and its transport
#   kube, kube-client          a Kubernetes API client
#   k8s-openapi                Kubernetes API types, and a very large one
#   aws-sdk-s3, aws-config     object storage over the network
#   axum, tower, tower-http    an HTTP server and its middleware
#   rustls                     TLS, therefore sockets
#   getrandom, rand            a host entropy source; breaks the wasm target
#   ureq, smol, async-std,     other runtimes and blocking HTTP clients
#   async-io
#
# As in ci/tls-provider-assertions.sh, each crate's graph is resolved once and
# then inspected, and each check refuses to report a pass when its own inputs are
# missing. Reading `cargo tree -i <dep>`'s exit code would report a clean
# boundary when `cargo` itself failed to run.
set -euo pipefail

cd "$(dirname "$0")/.."

RECORD="ci/pure-crate-dependencies.txt"
PURE_CRATES=(vf-core vf-authz vf-graph vf-translator)

MODE="check"
if [[ "${1:-}" == "--record" ]]; then
  MODE="record"
elif [[ $# -gt 0 ]]; then
  echo "usage: $0 [--record]" >&2
  exit 2
fi

command -v cargo >/dev/null 2>&1 || {
  echo "FAIL: cargo is not on PATH; this gate cannot assert anything" >&2
  exit 1
}

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# The complete transitive set a pure crate reaches, one name per line.
resolve() {
  local crate="$1"
  if ! cargo tree --locked --offline -p "$crate" --edges normal --target all \
       --prefix none > "$WORK/$crate.tree" 2> "$WORK/$crate.err"; then
    echo "FAIL: could not resolve the dependency graph for ${crate}:" >&2
    cat "$WORK/$crate.err" >&2
    exit 1
  fi
  awk '{ if ($2 ~ /^v[0-9]/) print $1 }' "$WORK/$crate.tree" | sort -u
}

# The recorded set for one crate, read from the `[crate]` section of $RECORD.
recorded() {
  awk -v want="[$1]" '
    /^\[/ { in_section = ($0 == want); next }
    in_section && NF && $0 !~ /^#/ { print }
  ' "$RECORD" | sort -u
}

# --- --record: regenerate the file ---------------------------------------------
if [[ "$MODE" == "record" ]]; then
  {
    cat <<'HEADER'
# The complete expected transitive dependency set of each pure library crate,
# one crate name per line under a `[crate]` heading.
#
# This file is the mechanism behind ci/crate-boundaries.sh: the gate resolves
# each crate and diffs the result against the section below, so a dependency
# nobody anticipated fails the build without being named in the script. See that
# script's header for why it is an allowlist and why names carry no version.
#
# Regenerate with `ci/crate-boundaries.sh --record`, and read the diff before
# committing it. A new line here is a new crate inside a safety-critical,
# I/O-free, WebAssembly-targeting library — which is a review conversation, not
# a formatting change.
HEADER
    for crate in "${PURE_CRATES[@]}"; do
      printf '\n[%s]\n' "$crate"
      resolve "$crate"
    done
  } > "$WORK/record.txt"
  mv "$WORK/record.txt" "$RECORD"
  echo "recorded the transitive set of ${PURE_CRATES[*]} in ${RECORD}"
  exit 0
fi

# --- check --------------------------------------------------------------------
test -s "$RECORD" || {
  echo "FAIL: ${RECORD} is missing or empty; this gate has nothing to diff against" >&2
  exit 1
}

failed=0
for crate in "${PURE_CRATES[@]}"; do
  resolve "$crate" > "$WORK/$crate.resolved"
  recorded "$crate" > "$WORK/$crate.recorded"

  test -s "$WORK/$crate.resolved" || {
    echo "FAIL: the resolved graph for ${crate} is empty; refusing to report a pass" >&2
    exit 1
  }
  test -s "$WORK/$crate.recorded" || {
    echo "FAIL: ${RECORD} has no [${crate}] section; refusing to report a pass" >&2
    exit 1
  }

  if diff -u "$WORK/$crate.recorded" "$WORK/$crate.resolved" \
       > "$WORK/$crate.diff" 2>&1; then
    echo "ok   ${crate}: $(wc -l < "$WORK/$crate.resolved") transitive crates, all recorded"
    continue
  fi

  failed=1
  {
    echo "FAIL ${crate}: its transitive set differs from ${RECORD}"
    sed 's/^/     /' "$WORK/$crate.diff"

    # Arrivals, with a reason where this script happens to know one. The
    # addition itself is the failure; the reason is a courtesy.
    comm -13 "$WORK/$crate.recorded" "$WORK/$crate.resolved" | while read -r added; do
      case "$added" in
        tokio|mio|socket2)        reason="an async runtime or its syscall layer" ;;
        sqlx)                     reason="a database client" ;;
        reqwest|hyper|hyper-util) reason="an HTTP client or its transport" ;;
        kube|kube-client)         reason="a Kubernetes API client" ;;
        k8s-openapi)              reason="Kubernetes API types" ;;
        aws-sdk-s3|aws-config)    reason="object storage over the network" ;;
        axum|tower|tower-http)    reason="an HTTP server or its middleware" ;;
        rustls)                   reason="TLS, therefore sockets" ;;
        getrandom|rand)           reason="a host entropy source; breaks wasm32-unknown-unknown" ;;
        ureq|smol|async-std|async-io) reason="another runtime or blocking HTTP client" ;;
        *)                        reason="" ;;
      esac
      if [[ -n "$reason" ]]; then
        echo "     + ${added} — ${reason}"
      else
        echo "     + ${added}"
      fi
      cargo tree --locked --offline -p "$crate" --edges normal --target all \
        -i "$added" 2>/dev/null | sed 's/^/       /' || true
    done

    echo
    echo "     If this crate genuinely should reach these, record it deliberately:"
    echo "       ci/crate-boundaries.sh --record"
    echo "     and put the resulting ${RECORD} diff in front of a reviewer."
  } >&2
done

if [[ "$failed" -ne 0 ]]; then
  echo "crate boundary assertion failed: a pure library crate's dependency set changed" >&2
  exit 1
fi
echo "crate boundaries: ${PURE_CRATES[*]} match their recorded, I/O-free dependency sets"
