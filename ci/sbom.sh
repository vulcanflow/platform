#!/usr/bin/env bash
# Generates a CycloneDX SBOM for the workspace (TDD §21.3).
#
# Two artifacts, because they answer different questions:
#
#   * `sbom/vulcanflow-workspace.cdx.json` — an aggregate CycloneDX document for
#     the whole workspace, resolved for the Aether target. This is what gets
#     attached to a release and fed to an advisory scanner.
#   * `sbom/<crate>.cdx.json` — one per crate, so an image that ships a single
#     binary can carry the SBOM for what is actually in it rather than for
#     everything the workspace can build.
#
# §21.3 says "an SBOM per image"; images are not built here (VUL-6 is code and CI
# only, no cluster), so this produces the documents the image build will embed.
set -euo pipefail

cd "$(dirname "$0")/.."
# shellcheck source=./tool-versions.env
source ci/tool-versions.env

OUT_DIR="${1:-target/sbom}"
rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"

cargo cyclonedx \
  --locked \
  --all-features \
  --target "${AETHER_TARGET}" \
  --format json \
  --spec-version 1.5 \
  --output-cdx \
  --output-pattern package \
  --output-prefix "$OUT_DIR/"

# cargo-cyclonedx writes next to each manifest unless told otherwise; normalise
# whatever landed into OUT_DIR so the CI upload step has one place to look.
find . -name '*.cdx.json' -not -path './target/*' -print0 \
  | xargs -0 -r -I{} mv {} "$OUT_DIR/"

count="$(find "$OUT_DIR" -name '*.cdx.json' | wc -l)"
if [[ "$count" -eq 0 ]]; then
  echo "FAIL: no SBOM documents were produced (TDD §21.3)" >&2
  exit 1
fi
echo "build/rust-supply-chain: ${count} CycloneDX SBOM document(s) in ${OUT_DIR}"
