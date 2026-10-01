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
# As in ci/tls-provider-assertions.sh, each crate's graph is resolved once and
# then inspected. Reading `cargo tree -i <dep>`'s exit code would report a clean
# boundary when `cargo` itself failed to run.
set -euo pipefail

cd "$(dirname "$0")/.."

command -v cargo >/dev/null 2>&1 || {
  echo "FAIL: cargo is not on PATH; this gate cannot assert anything" >&2
  exit 1
}

PURE_CRATES=(vf-core vf-authz vf-graph vf-translator)
# Transitive presence of any of these means the crate performs I/O, carries an
# async runtime, or needs a host environment.
FORBIDDEN=(tokio sqlx reqwest kube kube-client k8s-openapi hyper hyper-util
           aws-sdk-s3 aws-config axum tower tower-http rustls mio socket2)

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

failed=0
for crate in "${PURE_CRATES[@]}"; do
  # `--edges normal` on purpose: the test authors' harnesses may use a runtime,
  # the shipped library may not.
  if ! cargo tree --locked --offline -p "$crate" --edges normal --prefix none --no-dedupe \
       > "$WORK/$crate.txt" 2> "$WORK/$crate.err"; then
    echo "FAIL: could not resolve the dependency graph for ${crate}:" >&2
    cat "$WORK/$crate.err" >&2
    exit 1
  fi
  awk '{ if ($2 ~ /^v[0-9]/) print $1 }' "$WORK/$crate.txt" | sort -u > "$WORK/$crate.names"
  test -s "$WORK/$crate.names" || { echo "FAIL: empty graph for ${crate}" >&2; exit 1; }

  hits=()
  for dep in "${FORBIDDEN[@]}"; do
    if grep -qx -- "$dep" "$WORK/$crate.names"; then hits+=("$dep"); fi
  done

  if [[ ${#hits[@]} -gt 0 ]]; then
    {
      echo "FAIL ${crate} reaches I/O crates: ${hits[*]}"
      for dep in "${hits[@]}"; do
        cargo tree --locked --offline -p "$crate" --edges normal -i "$dep" 2>/dev/null | sed 's/^/     /' || true
      done
    } >&2
    failed=1
  else
    echo "ok   ${crate}: $(wc -l < "$WORK/$crate.names") transitive crates, none of them I/O"
  fi
done

if [[ "$failed" -ne 0 ]]; then
  echo "crate boundary assertion failed: a pure library crate reached an I/O crate" >&2
  exit 1
fi
echo "crate boundaries: ${PURE_CRATES[*]} are I/O-free"
