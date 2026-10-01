#!/usr/bin/env bash
# Generates CycloneDX SBOMs for the workspace (TDD §21.3).
#
# `cargo cyclonedx` writes one BOM next to every `Cargo.toml` in the workspace,
# so this runs it and then collects the results into a single directory for the
# CI upload step, renaming each one after the crate it describes.
#
# §21.3 says "an SBOM per image"; images are not built here (VUL-6 is code and
# CI only, no cluster), so this produces the documents the image build will
# embed.
#
# Default features, NOT `--all-features`. An SBOM is an inventory of what
# shipped. `--all-features` enables optional dependencies no release build
# compiles — `rustls/ring` among them — so the document would list crates that
# are not in the artifact and omit nothing in exchange. A false inventory is
# worse than no inventory, because an advisory scanner reads it as fact. Same
# reasoning as `all-features = false` in deny.toml.
#
# `--target aarch64-unknown-linux-gnu`, not the host, for the same reason: an
# SBOM for a platform we do not ship describes something that does not exist.
#
# cargo-cyclonedx takes neither `--locked` nor `--offline`. The lockfile is
# asserted current by the preceding step in the workflow.
set -euo pipefail

cd "$(dirname "$0")/.."
# shellcheck source=./tool-versions.env
source ci/tool-versions.env

command -v cargo-cyclonedx >/dev/null 2>&1 || {
  echo "FAIL: cargo-cyclonedx is not installed; refusing to report an SBOM" >&2
  exit 1
}

OUT_DIR="${1:-target/sbom}"
rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"

# Clear any stale BOMs first, so an interrupted previous run's output cannot be
# collected below and reported as this one's.
find . -path ./target -prune -o \( -name 'bom.json' -o -name 'bom.xml' \) -print0 \
  | xargs -0 -r rm -f

cargo cyclonedx \
  --format json \
  --spec-version 1.5 \
  --target "${AETHER_TARGET}" \
  --all

count=0
while IFS= read -r -d '' bom; do
  crate="$(basename "$(dirname "$bom")")"
  mv "$bom" "${OUT_DIR}/${crate}.cdx.json"
  count=$((count + 1))
done < <(find . -path ./target -prune -o -name 'bom.json' -print0)

if [[ "$count" -eq 0 ]]; then
  echo "FAIL: no SBOM documents were produced (TDD §21.3)" >&2
  exit 1
fi
echo "build/rust-supply-chain: ${count} CycloneDX 1.5 SBOM document(s) in ${OUT_DIR}"
ls -1 "$OUT_DIR"
