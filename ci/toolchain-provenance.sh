#!/usr/bin/env bash
# toolchain-provenance.sh — refuse a cargo that is not the pinned toolchain.
#
# The cargo recipes in the justfile run whatever `cargo` is first on PATH. A
# caller who skipped the bootstrap, or whose eval of its env silently failed,
# would otherwise build with a system cargo of another version and never be
# told (VFL-909). This runs first, as a `just` dependency of the
# stable-toolchain cargo recipes, and fails closed:
#
#   1. `cargo --version` and `rustc --version`, resolved on PATH from the repo
#      root, must report the channel rust-toolchain.toml pins. The channel is
#      read from that file, which stays the one place it is written. A rustup
#      proxy passes as is: it selects the toolchain from that same file.
#
#   2. When VF_TOOLCHAIN_DIR is set, the bootstrap's env is meant to be in
#      force, so both must also resolve in $CARGO_HOME/bin, with CARGO_HOME
#      inside that prefix. When it is unset — a rustup-managed developer
#      machine or CI runner — only check 1 applies.
#
# It changes nothing. On success it prints nothing of its own; on a failure it
# prints one line to stderr and exits 1.
#
# Usage: ci/toolchain-provenance.sh

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

die() { printf 'toolchain-provenance: %s\n' "$1" >&2; exit 1; }

# The physical path of a directory, or nothing if it is not one. Both sides of
# every comparison go through this, so a symlink, a relative PATH entry or a
# doubled slash cannot make the same directory compare unequal. An empty
# argument is not one: `cd ''` stays where it is and would answer the repo root.
canon() { [ -n "$1" ] && (CDPATH='' cd -- "$1" 2>/dev/null && pwd -P) || true; }

[ -f rust-toolchain.toml ] || die "no rust-toolchain.toml in $repo_root"

# Exactly one whole x.y.z. A channel name (stable, nightly-…) or a partial
# version names no single release to compare with, so it is refused rather
# than matched loosely; two `channel` lines fail the same pattern.
channel="$(sed -nE 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"([^"]*)".*/\1/p' rust-toolchain.toml)"
[[ "$channel" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] \
  || die "rust-toolchain.toml must pin channel to one x.y.z version; found '${channel//$'\n'/, }'"

for tool in cargo rustc; do
  path="$(command -v "$tool")" || die "no $tool on PATH; rust-toolchain.toml pins channel $channel"
  # An exported shell function by that name is what a recipe would run
  # instead, and it is not a file whose version this can vouch for.
  [[ "$path" == */* ]] \
    || die "$tool resolves to a shell $(type -t "$tool"), not a file; rust-toolchain.toml pins channel $channel"

  # Only stdout is the version. stderr is left to the terminal: a rustup proxy
  # that is installing the pinned toolchain says so there first, and that
  # must not be read as the version.
  reported="$("$path" --version)" || reported="\`$tool --version\` failed"
  reported="${reported%%$'\n'*}"
  case "$reported" in
    "$tool $channel" | "$tool $channel "*) ;;
    *) die "$path reports '$reported', but rust-toolchain.toml pins channel $channel" ;;
  esac

  # Empty counts as unset, as it does for the bootstrap's own prefix default.
  if [ -n "${VF_TOOLCHAIN_DIR:-}" ]; then
    prefix="$(canon "$VF_TOOLCHAIN_DIR")"
    home="$(canon "${CARGO_HOME:-}")"
    if [ -z "$prefix" ] || [ -z "$home" ] || [[ "$home" != "${prefix%/}/"* ]] \
      || [ "$(canon "${path%/*}")" != "$(canon "$home/bin")" ]; then
      die "$path reports '$reported' (pinned channel $channel), but VF_TOOLCHAIN_DIR=$VF_TOOLCHAIN_DIR wants it in \$CARGO_HOME/bin inside that prefix (CARGO_HOME=${CARGO_HOME:-<unset>})"
    fi
  fi
done
