#!/usr/bin/env bash
# graph-rules.sh — the §A1.4 crate-graph rules that deny.toml cannot express.
#
# Most of §A1.4 is enforced as `bans.wrappers` entries in deny.toml. Two rules
# are not expressible there and live here instead:
#
#   1. vf-core and vf-graph depend on no I/O crate. Not a ban, because the
#      legitimate direct dependents of tokio/sqlx/reqwest/kube are third-party
#      crates whose set changes on every bump, so the wrappers list would be
#      unmaintainable and would have to be widened rather than reviewed.
#
#   2. No binary crate depends on another binary crate. Not a ban, because a
#      cargo-deny ban talks about a crate's presence in the graph, not about
#      the edges into it: `wrappers = []` bans the crate outright.
#
# Run by `just graph-rules`, and by `just gate` before a push.
#
# Usage: ci/graph-rules.sh

set -euo pipefail

fail() { printf '\033[31mFAIL\033[0m %s\n' "$1" >&2; status=1; }
pass() { printf '\033[32mok\033[0m   %s\n' "$1"; }

status=0

# The crates that must stay I/O free, with the target each is checked on.
# vf-graph is checked on wasm32 because that is the build whose bundle size and
# browser portability the rule protects.
io_free_crates=(
  "vf-core|"
  "vf-graph|--target wasm32-unknown-unknown"
)

# Direct markers of I/O. Deliberately a short, explicit list rather than a
# category: each of these is named in §A1.4 or is the transport underneath one.
io_crates=(tokio sqlx reqwest kube hyper object_store redis)

# ---------------------------------------------------------------------------
# 1. vf-core and vf-graph depend on no I/O crate
# ---------------------------------------------------------------------------

check_io_free() {
  local crate="$1" extra="$2" tree hits=()

  # shellcheck disable=SC2086 # $extra is a deliberate argument list
  tree="$(cargo tree -p "$crate" $extra --prefix none --no-dedupe)"

  local io
  for io in "${io_crates[@]}"; do
    if grep -qE "^${io} v" <<<"$tree"; then
      hits+=("$io")
    fi
  done

  if [ ${#hits[@]} -gt 0 ]; then
    fail "$crate depends on I/O crates: ${hits[*]} (§A1.4)"
  else
    pass "$crate: no I/O crate"
  fi
}

for entry in "${io_free_crates[@]}"; do
  check_io_free "${entry%%|*}" "${entry#*|}"
done

# ---------------------------------------------------------------------------
# 2. No binary crate depends on another binary crate
#
# A workspace member counts as a binary if it has a bin target. vf-api also has
# a lib target, and cargo records a dependency by package name whichever target
# is used, so this check is deliberately strict: a package with a bin target may
# not appear in another member's normal dependency list at all. A binary that
# needs another's logic takes the dependency on a library crate instead.
#
# Dev-dependencies are exempt (kind "dev"), so the test lane can drive a binary.
# ---------------------------------------------------------------------------

offenders="$(cargo metadata --no-deps --format-version 1 | python3 -c '
import json, sys

meta = json.load(sys.stdin)
packages = meta["packages"]
bins = {p["name"] for p in packages if any(t["kind"] == ["bin"] for t in p["targets"])}

for p in packages:
    for d in p["dependencies"]:
        # kind is None for a normal dependency, "dev" or "build" otherwise.
        if d["name"] in bins and d["kind"] is None:
            print("{} -> {}".format(p["name"], d["name"]))
')"

if [ -n "$offenders" ]; then
  fail "binary crate depends on another binary crate (§A1.4):"
  printf '      %s\n' $offenders >&2
else
  pass "no binary crate depends on another binary crate"
fi

exit "$status"
