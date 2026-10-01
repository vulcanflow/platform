#!/usr/bin/env bash
# Builds every release binary for the Aether target twice, in two different
# directories, and asserts the artifacts are byte-identical (TDD §21.3).
#
# Usage: ci/reproducible-build.sh [output-dir]
#
# What actually makes a Rust build reproducible here, and why each piece is
# present:
#
#   * The toolchain is pinned in rust-toolchain.toml and the dependency versions
#     in Cargo.lock. `--locked` fails rather than silently updating either.
#   * `--offline` after a single `cargo fetch`: a build that reaches the network
#     is not reproducible, and it also hides a lockfile that does not match.
#   * `--remap-path-prefix` for the build directory, the registry source cache
#     and the toolchain sysroot. Absolute paths leak into binaries through panic
#     messages and `file!()`, and they differ between a CI runner and a laptop.
#   * `codegen-units = 1`, `incremental = false`, `lto = "thin"` in the release
#     profile: parallel codegen partitioning is not deterministic.
#   * SOURCE_DATE_EPOCH, taken from the commit being built rather than from the
#     wall clock.
#
# The two builds use different CARGO_TARGET_DIRs on purpose. Building twice into
# the same directory proves only that cargo cached the first result.
set -euo pipefail

cd "$(dirname "$0")/.."
# shellcheck source=./tool-versions.env
source ci/tool-versions.env

OUT_DIR="${1:-target/reproducibility}"
TARGET="${AETHER_TARGET}"

# Deterministic timestamp: the commit date, not `date`.
if [[ -z "${SOURCE_DATE_EPOCH:-}" ]]; then
  SOURCE_DATE_EPOCH="$(git log -1 --pretty=%ct 2>/dev/null || echo 0)"
fi
export SOURCE_DATE_EPOCH

REGISTRY_SRC="${CARGO_HOME:-$HOME/.cargo}/registry/src"
SYSROOT="$(rustc --print sysroot)"

build_once() {
  local build_dir="$1"
  local target_dir="$2"

  rm -rf "$build_dir"
  mkdir -p "$(dirname "$build_dir")"
  # A fresh copy at a different absolute path. If any path leaks into an
  # artifact, the two builds disagree and this script is the thing that says so.
  git ls-files -z | xargs -0 -I{} install -D "{}" "$build_dir/{}"
  cp Cargo.lock "$build_dir/Cargo.lock" 2>/dev/null || true

  (
    cd "$build_dir"
    CARGO_TARGET_DIR="$target_dir" \
    RUSTFLAGS="--remap-path-prefix=${build_dir}=/vulcanflow \
--remap-path-prefix=${REGISTRY_SRC}=/cargo-registry \
--remap-path-prefix=${SYSROOT}=/rust-sysroot" \
      cargo build --locked --offline --release --workspace --bins --target "$TARGET"
  )
}

digest_tree() {
  local target_dir="$1"
  find "$target_dir/$TARGET/release" -maxdepth 1 -type f -executable -printf '%f\n' \
    | sort \
    | while read -r bin; do
        printf '%s  %s\n' "$(sha256sum "$target_dir/$TARGET/release/$bin" | cut -d' ' -f1)" "$bin"
      done
}

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "== build 1 =="
build_once "$WORK/a/vulcanflow" "$WORK/a-target"
echo "== build 2 =="
build_once "$WORK/b/some/deeper/path/vulcanflow" "$WORK/b-target"

mkdir -p "$OUT_DIR"
digest_tree "$WORK/a-target" > "$OUT_DIR/digests-1.txt"
digest_tree "$WORK/b-target" > "$OUT_DIR/digests-2.txt"

echo "== digests =="
cat "$OUT_DIR/digests-1.txt"

if ! diff -u "$OUT_DIR/digests-1.txt" "$OUT_DIR/digests-2.txt"; then
  echo "FAIL: the arm64 release build is not reproducible (TDD §21.3)" >&2
  exit 1
fi

if [[ ! -s "$OUT_DIR/digests-1.txt" ]]; then
  echo "FAIL: no release binaries were produced; the comparison proved nothing" >&2
  exit 1
fi

echo "build/rust-supply-chain: arm64 release build is reproducible across two builds"
