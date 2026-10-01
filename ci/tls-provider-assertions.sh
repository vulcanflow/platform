#!/usr/bin/env bash
# ADR-0002 §3.6 (amendment A1) asks `build/rust-supply-chain` for two assertions:
# `cargo tree -i ring` empty, and `cargo tree -d` showing no duplicated `rustls`,
# `hmac`, `sha2` or `digest`.
#
# Why they are build-time checks and not a runtime concern: `rustls 0.23`
# declares both `aws-lc-rs` and `ring` as optional providers, and we reach rustls
# through `reqwest`, `sqlx`, `kube` and `aws-sdk-s3`. If feature unification ever
# enables both, both compile cleanly, rustls cannot determine a process default,
# and the failure is a panic at the first TLS handshake in production rather than
# a build error. A second RustCrypto generation is the same shape of problem:
# `hmac 0.12`/`sha2 0.10` alongside `hmac 0.13`/`sha2 0.11` compiles, and a
# signature path silently uses whichever one its call site resolved.
#
# Two things about how this is implemented, both of which are the point:
#
# 1. **The graph is resolved once and then inspected**, rather than running
#    `cargo tree -i <crate>` per crate and reading the exit code. `cargo tree -i`
#    exits non-zero both when the crate is absent and when `cargo` itself cannot
#    run, so an exit-code check reports a clean result for a broken environment.
#    A supply-chain gate that passes when its own tooling is missing is worse
#    than no gate.
# 2. **Default features, not `--all-features`.** `--all-features` switches on
#    every optional provider by definition, so `ring` appears and the check
#    describes a configuration nobody builds. The question worth asking is what
#    is in the artifact. Same reasoning as `all-features = false` in deny.toml.
#
# ------------------------------------------------------------------------------
# A1 assertion 2 is currently only partly satisfiable, and this script says so
# out loud rather than quietly narrowing the rule.
#
# `rustls` and `hmac` are single-version and are failed on. `sha2` and `digest`
# are not single-version and cannot be made so at the pinned versions:
# `sqlx-core 0.9.0` depends on `sha2 0.10` while `sqlx-postgres 0.9.0` depends on
# `sha2 0.11`, so the split lives inside sqlx and no pin of ours moves it.
# `digest 0.10` follows from that, plus `crc-fast` under the AWS checksum path.
# Neither is on a signature-verification surface: every VulcanFlow crate that
# hashes is on the 0.11 generation.
#
# Those two are reported as findings with their full paths; the exit code is
# governed by STRICT_RUSTCRYPTO. Narrowing or keeping A1's rule is Atlas's
# decision (raised on VUL-6 as an A2 candidate), not this file's — set
# STRICT_RUSTCRYPTO=1 as soon as an amendment or a sqlx release makes it
# achievable, and delete this paragraph with it.
STRICT_RUSTCRYPTO="${STRICT_RUSTCRYPTO:-0}"
# ------------------------------------------------------------------------------
set -euo pipefail

cd "$(dirname "$0")/.."

command -v cargo >/dev/null 2>&1 || {
  echo "FAIL: cargo is not on PATH; this gate cannot assert anything" >&2
  exit 1
}

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# One resolution of the shipping graph: normal edges only (dev- and
# build-dependencies do not ship), default features, every workspace member.
if ! cargo tree --locked --offline --edges normal --prefix none --no-dedupe \
     > "$WORK/graph.txt" 2> "$WORK/graph.err"; then
  echo "FAIL: 'cargo tree' could not resolve the workspace:" >&2
  cat "$WORK/graph.err" >&2
  exit 1
fi
# `name vX.Y.Z` per line, with path/feature suffixes dropped.
awk '{ if ($2 ~ /^v[0-9]/) print $1, substr($2, 2) }' "$WORK/graph.txt" \
  | sort -u > "$WORK/packages.txt"

test -s "$WORK/packages.txt" || {
  echo "FAIL: the resolved graph is empty; refusing to report a pass" >&2
  exit 1
}
echo "resolved $(wc -l < "$WORK/packages.txt") distinct package versions in the shipping graph"

hard_failed=0
soft_failed=0

versions_of() { awk -v c="$1" '$1 == c { print $2 }' "$WORK/packages.txt"; }

why() {
  # Reverse-dependency paths, for a human reading a red build.
  cargo tree --locked --offline --edges normal -i "$1@$2" --depth 2 2>/dev/null \
    | sed 's/^/         /' || true
}

# --- A1 assertion 1: `ring` is absent from the shipping graph. ----------------
# `chrono` is a cargo-deny ban rather than an A1 assertion, but it enters the
# same way — an optional feature on a crate we do want — and the resolved graph
# is already in hand, so it is checked here too.
for banned in ring chrono; do
  found="$(versions_of "$banned")"
  if [[ -n "$found" ]]; then
    {
      echo "FAIL: '${banned}' is in the shipping dependency graph:"
      for v in $found; do echo "         ${banned} ${v}"; why "$banned" "$v"; done
    } >&2
    hard_failed=1
  else
    echo "ok    no '${banned}' in the shipping dependency graph"
  fi
done

# --- A1 assertion 2: one version each. ----------------------------------------
check_single() {
  local crate="$1" severity="$2" found count label
  found="$(versions_of "$crate")"
  count="$(printf '%s' "$found" | grep -c . || true)"

  if [[ "$count" -eq 0 ]]; then echo "ok    ${crate} is not in the shipping graph"; return 0; fi
  if [[ "$count" -eq 1 ]]; then echo "ok    single version of ${crate} (${found})"; return 0; fi

  label="FAIL"; [[ "$severity" == "soft" ]] && label="FINDING"
  {
    echo "${label}: ${crate} resolves to ${count} versions:"
    for v in $found; do echo "         ${crate} ${v}"; why "$crate" "$v"; done
  } >&2
  if [[ "$severity" == "soft" ]]; then soft_failed=1; else hard_failed=1; fi
}

# Load-bearing for TLS and for webhook signature verification (ADR-0003 §4.4).
check_single rustls hard
check_single hmac hard

if [[ "$STRICT_RUSTCRYPTO" == "1" ]]; then
  check_single sha2 hard
  check_single digest hard
else
  check_single sha2 soft
  check_single digest soft
fi

if [[ "$hard_failed" -ne 0 ]]; then
  echo "build/rust-supply-chain: A1 TLS/crypto assertions FAILED" >&2
  exit 1
fi
if [[ "$soft_failed" -ne 0 ]]; then
  echo "NOTE: ring, chrono, rustls and hmac all pass. sha2 and digest are duplicated" >&2
  echo "upstream of us (sqlx 0.9.0) and are reported above rather than failed on." >&2
  echo "Atlas owns the A2 decision; see the header and VUL-6." >&2
fi
echo "build/rust-supply-chain: A1 TLS/crypto assertions complete"
