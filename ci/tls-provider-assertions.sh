#!/usr/bin/env bash
# ADR-0002 §3.7.9 — amendment A1's second assertion as amendment A2 restated it.
#
# The rule, quoted so a reader does not have to fetch the ADR:
#
#   Evaluated per shipped binary (`cargo tree -p <bin> --edges
#   normal,no-proc-macro`, default features), the graph must contain no `ring`,
#   no `chrono`, and exactly one version each of `rustls`, `hmac` and `sha2`.
#   `digest` may resolve to two versions only when the second is `0.10.x`
#   reachable solely through `crc-fast` <- `aws-smithy-checksums`; the gate fails
#   on `digest 0.10` arriving by any other path.
#
# Why these are build-time checks and not a runtime concern: `rustls 0.23`
# declares both `aws-lc-rs` and `ring` as optional providers, and we reach rustls
# through `reqwest`, `sqlx`, `kube` and `aws-sdk-s3`. If feature unification ever
# enables both, both compile cleanly, rustls cannot determine a process default,
# and the failure is a panic at the first TLS handshake in production rather than
# a build error. A second RustCrypto generation is the same shape of problem:
# `hmac 0.12`/`sha2 0.10` alongside `hmac 0.13`/`sha2 0.11` compiles, and a
# signature path silently uses whichever one its call site resolved.
#
# Four things about how this is implemented, all of them the point:
#
# 1. **Per shipped binary, not per workspace.** What ships is a binary built with
#    its own feature set. A workspace-wide resolution unifies features across
#    members and reports duplicates that are in no artifact. The binary list is
#    derived from `cargo metadata`, so a new binary crate is covered the day it
#    is added rather than the day someone remembers to edit this file.
# 2. **`no-proc-macro`.** A hash run on the build host at compile time is not a
#    second implementation in the binary. Counting it makes the gate wrong in the
#    direction that gets gates deleted.
# 3. **The `digest` exception is asserted by path, not waived by version.** "Two
#    versions are fine" would admit a whole second RustCrypto stack through any
#    new dependency. "Two versions are fine iff the second is reachable only
#    here" still fails on the thing A1 was built to catch. So the script
#    reconstructs the parent edges of the resolved tree and checks them.
# 4. **No escape hatch.** There is no environment variable that relaxes any of
#    this. A supply-chain assertion with an off switch is an assertion that is
#    off. Narrowing the rule is an ADR amendment, which is how §3.7.9 itself
#    came about.
#
# Each check refuses to report a pass when its own inputs are missing. A
# supply-chain gate that goes green because `cargo` could not run is worse than
# no gate.
#
# NOTE on `vf-graph`: it is shipped to the browser as well (TDD §7.3, §14.1) and
# is not a binary, so it is not covered here. ci/crate-boundaries.sh asserts its
# complete transitive set against a recorded allowlist, which is strictly
# stronger than the four names below.
set -euo pipefail

cd "$(dirname "$0")/.."

command -v cargo >/dev/null 2>&1 || {
  echo "FAIL: cargo is not on PATH; this gate cannot assert anything" >&2
  exit 1
}
command -v python3 >/dev/null 2>&1 || {
  echo "FAIL: python3 is not on PATH; the shipped-binary list cannot be read" >&2
  exit 1
}

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# --- which packages actually ship as a binary ---------------------------------
# `--no-deps` keeps this to workspace members. A member counts if it has at least
# one target of kind `bin`.
if ! cargo metadata --locked --offline --format-version 1 --no-deps \
     > "$WORK/metadata.json" 2> "$WORK/metadata.err"; then
  echo "FAIL: 'cargo metadata' could not read the workspace:" >&2
  cat "$WORK/metadata.err" >&2
  exit 1
fi
python3 - "$WORK/metadata.json" > "$WORK/bins.txt" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as handle:
    metadata = json.load(handle)
for package in metadata["packages"]:
    if any("bin" in target["kind"] for target in package["targets"]):
        print(package["name"])
PY
sort -u -o "$WORK/bins.txt" "$WORK/bins.txt"

test -s "$WORK/bins.txt" || {
  echo "FAIL: no binary crates found in the workspace; refusing to report a pass" >&2
  exit 1
}
echo "shipped binaries under assertion: $(tr '\n' ' ' < "$WORK/bins.txt")"

# Reconstructs the parent edges of a `cargo tree --prefix depth` listing.
#
# Each line is `<depth><name> v<version>[ (source)][ (*)]`. A node's parent is
# the nearest preceding line one level shallower, which a depth-indexed stack
# gives directly. `(*)` marks a node whose children were printed elsewhere in the
# tree, so the union of edges over the whole listing is complete even though no
# subtree is printed twice.
EDGES_AWK='
{
  line = $0
  match(line, /^[0-9]+/)
  depth = substr(line, 1, RLENGTH) + 0
  rest  = substr(line, RLENGTH + 1)
  split(rest, field, " ")
  node = field[1] "@" substr(field[2], 2)
  stack[depth] = node
  print node > nodes
  if (depth > 0) print stack[depth - 1] "\t" node
}'

failed=0

for bin in $(cat "$WORK/bins.txt"); do
  if ! cargo tree --locked --offline -p "$bin" --edges normal,no-proc-macro \
       --prefix depth > "$WORK/$bin.tree" 2> "$WORK/$bin.err"; then
    echo "FAIL: 'cargo tree' could not resolve ${bin}:" >&2
    cat "$WORK/$bin.err" >&2
    exit 1
  fi
  awk -v nodes="$WORK/$bin.nodes" "$EDGES_AWK" "$WORK/$bin.tree" \
    | sort -u > "$WORK/$bin.edges"
  sort -u -o "$WORK/$bin.nodes" "$WORK/$bin.nodes"

  test -s "$WORK/$bin.nodes" || {
    echo "FAIL: the resolved graph for ${bin} is empty; refusing to report a pass" >&2
    exit 1
  }

  bin_failed=0
  problems=()

  versions_of() { sed -n "s/^$1@//p" "$WORK/$bin.nodes"; }
  parents_of()  { awk -F'\t' -v c="$1" '$2 == c { print $1 }' "$WORK/$bin.edges" \
                    | sed 's/@.*//' | sort -u; }
  why() {
    cargo tree --locked --offline -p "$bin" --edges normal,no-proc-macro \
      -i "$1" 2>/dev/null | sed 's/^/           /' || true
  }

  # --- no `ring`, no `chrono` -------------------------------------------------
  # `chrono` is a cargo-deny ban (ADR-0002 §3.5) rather than an A1 assertion, but
  # §3.7.9 names it and it enters the same way — an optional feature on a crate we
  # do want — so it is checked on the graph that is already in hand.
  for banned in ring chrono; do
    found="$(versions_of "$banned")"
    if [[ -n "$found" ]]; then
      problems+=("'${banned}' is in the shipping graph of ${bin}:")
      for v in $found; do
        problems+=("         ${banned} ${v}")
        problems+=("$(why "${banned}@${v}")")
      done
      bin_failed=1
    fi
  done

  # --- exactly one `rustls`, one `hmac`, one `sha2` ---------------------------
  for crate in rustls hmac sha2; do
    found="$(versions_of "$crate")"
    count="$(printf '%s' "$found" | grep -c . || true)"
    if [[ "$count" -gt 1 ]]; then
      problems+=("${crate} resolves to ${count} versions in ${bin}:")
      for v in $found; do
        problems+=("         ${crate} ${v}")
        problems+=("$(why "${crate}@${v}")")
      done
      bin_failed=1
    fi
  done

  # --- `digest`: a second version only by the one permitted path -------------
  digest_versions="$(versions_of digest)"
  digest_count="$(printf '%s' "$digest_versions" | grep -c . || true)"
  if [[ "$digest_count" -gt 1 ]]; then
    for v in $digest_versions; do
      case "$v" in
        0.11.*) continue ;;
        0.10.*)
          # Permitted only if `crc-fast` is its sole parent and
          # `aws-smithy-checksums` is `crc-fast`'s sole parent.
          digest_parents="$(parents_of "digest@$v" | tr '\n' ' ')"
          crc_parents="$(parents_of "$(grep -m1 '^crc-fast@' "$WORK/$bin.nodes")" | tr '\n' ' ')"
          if [[ "$digest_parents" != "crc-fast " || "$crc_parents" != "aws-smithy-checksums " ]]; then
            problems+=("digest ${v} does not arrive by the only path ADR-0002 §3.7.9 permits.")
            problems+=("         permitted: crc-fast <- aws-smithy-checksums")
            problems+=("         parents of digest ${v}: ${digest_parents:-none}")
            problems+=("         parents of crc-fast:    ${crc_parents:-none (crc-fast absent)}")
            problems+=("$(why "digest@$v")")
            bin_failed=1
          fi
          ;;
        *)
          problems+=("digest ${v} is neither our 0.11 generation nor the 0.10.x")
          problems+=("         exception ADR-0002 §3.7.9 permits.")
          problems+=("$(why "digest@$v")")
          bin_failed=1
          ;;
      esac
    done
  fi

  if [[ "$bin_failed" -ne 0 ]]; then
    { echo "FAIL ${bin}"; printf '  %s\n' "${problems[@]}"; } >&2
    failed=1
  else
    echo "ok   ${bin}: $(wc -l < "$WORK/$bin.nodes") crates, one TLS provider, one RustCrypto generation"
  fi
done

if [[ "$failed" -ne 0 ]]; then
  echo "build/rust-supply-chain: ADR-0002 §3.7.9 TLS/crypto assertions FAILED" >&2
  exit 1
fi
echo "build/rust-supply-chain: ADR-0002 §3.7.9 TLS/crypto assertions pass for every shipped binary"
