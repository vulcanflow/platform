#!/usr/bin/env bash
# Asserts `#![forbid(unsafe_code)]` is present in every VulcanFlow crate root.
#
# The workspace lint table already sets `unsafe_code = "forbid"`, so this is
# belt-and-braces on purpose (ADR-0002 §2): the attribute survives a crate being
# vendored, moved, or built outside this workspace, and `build/rust-supply-chain`
# asserts the attribute, not the manifest.
set -euo pipefail

cd "$(dirname "$0")/.."

missing=0
for manifest in crates/*/Cargo.toml; do
  crate_dir="$(dirname "$manifest")"
  root=""
  for candidate in "$crate_dir/src/lib.rs" "$crate_dir/src/main.rs"; do
    [[ -f "$candidate" ]] && root="$candidate" && break
  done
  if [[ -z "$root" ]]; then
    echo "FAIL ${crate_dir}: no src/lib.rs or src/main.rs" >&2
    missing=1
    continue
  fi
  if ! grep -qE '^#!\[forbid\(unsafe_code\)\]' "$root"; then
    echo "FAIL ${root}: missing #![forbid(unsafe_code)]" >&2
    missing=1
    continue
  fi
  echo "ok   ${root}"
done

# A crate with both a lib and a bin needs the attribute in both roots; the loop
# above stops at lib.rs, so check any remaining bin roots too.
while IFS= read -r -d '' bin_root; do
  if ! grep -qE '^#!\[forbid\(unsafe_code\)\]' "$bin_root"; then
    echo "FAIL ${bin_root}: missing #![forbid(unsafe_code)]" >&2
    missing=1
  fi
done < <(find crates -path '*/src/bin/*.rs' -print0)

if [[ "$missing" -ne 0 ]]; then
  echo "build/rust-supply-chain: #![forbid(unsafe_code)] assertion failed" >&2
  exit 1
fi
echo "build/rust-supply-chain: #![forbid(unsafe_code)] present in every crate root"
