#!/usr/bin/env bash
# bootstrap-toolchain.sh — give a runner the toolchain `just check` needs.
#
# The agent runners ship `curl`, `python3` and `dpkg-deb`, but no C compiler,
# no linker, no libc headers and no Rust. Without a C toolchain nothing in this
# workspace compiles: `serde_derive` and `thiserror-impl` are proc macros, so
# `cargo check` builds their build scripts for the host and dies on
# `linker 'cc' not found` before it reaches any of our own code. See VFL-142.
#
# The runner is uid 65532 and cannot take the dpkg lock, so the toolchain is
# not installed — it is *unpacked*. Every package named in
# ci/toolchain/debs-<arch>.lock is fetched from the Debian archive, checked
# against the SHA256 recorded in that lock, and extracted into a private
# sysroot with `dpkg-deb -x`. No maintainer script runs, nothing outside the
# prefix is touched, and no privilege is needed beyond writing one directory.
#
# Two properties are worth stating because they are the reason this is safe
# rather than a hack:
#
#   1. The sysroot is self-contained. libc6-dev ships `libc.so` as a linker
#      script naming absolute paths, and ld resolves those relative to
#      --sysroot, so libc6 itself is inside the lock rather than borrowed from
#      the image. Nothing links against a library the lock does not pin.
#
#   2. libc6 and libc6-dev are pinned to the image's own glibc
#      (2.41-12+deb13u4). The ELF interpreter baked into a binary we build is
#      the unprefixed /lib/ld-linux-aarch64.so.1, so a built binary runs
#      against the image's loader. Pinning the pair to the image's version is
#      what keeps that from being a version skew.
#
# The lock is regenerated, not hand-edited: see ci/toolchain/resolve-debs.py.
#
# Usage:
#   eval "$(ci/bootstrap-toolchain.sh)"   # bootstrap, then export the env
#   ci/bootstrap-toolchain.sh --env-only  # just print the env, no network
#
# The prefix defaults to a sibling of the checkout so it survives `git clean`
# and is shared by every task on the same workspace. Override with
# VF_TOOLCHAIN_DIR. It is deliberately outside the work tree: it is 400 MiB of
# third-party binaries and must never be a candidate for commit.
#
# Because the prefix is shared, the C toolchain is kept per lock: the sysroot
# and the shims that point into it live under c-toolchain/<lock digest>/. A
# tree, once published, is never deleted by this script and its sysroot is
# never written again; only its shims are regenerated, each by a rename. A
# branch that changes the lock builds a new tree beside the old one, so it
# cannot pull a compiler out from under a run that is still building against
# the old lock. Trees for superseded locks are left in place; remove one by
# hand once nothing is using it.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
prefix="${VF_TOOLCHAIN_DIR:-$(cd "$repo_root/.." && pwd)/.vf-toolchain}"

cache_dir="$prefix/cache"
cargo_home="$prefix/cargo"
rustup_home="$prefix/rustup"

RUSTUP_VERSION="1.29.1"

# Architecture is detected, never assumed: the project is architecture
# agnostic and arm64 and amd64 are equal first-class targets (§A5).
# `loader` is spelled out per architecture rather than derived from `uname -m`:
# the two differ in both directory and suffix (/lib vs /lib64, .so.1 vs .so.2),
# so there is no substitution that produces both.
#
# Each case also names the *other* architecture, because the sysroot carries two
# compilers. §A5 has exactly two first-class Linux targets, so `just check-cross`
# means "the one that is not this machine" and the cross_ variables below are
# simply the host ones with the two architectures swapped.
case "$(uname -m)" in
  aarch64)
    deb_arch=arm64; gnu_triple=aarch64-linux-gnu; rust_host=aarch64-unknown-linux-gnu
    loader=/lib/ld-linux-aarch64.so.1
    cross_gnu_triple=x86_64-linux-gnu; cross_rust_target=x86_64-unknown-linux-gnu
    cross_loader=/lib64/ld-linux-x86-64.so.2 ;;
  x86_64)
    deb_arch=amd64; gnu_triple=x86_64-linux-gnu;  rust_host=x86_64-unknown-linux-gnu
    loader=/lib64/ld-linux-x86-64.so.2
    cross_gnu_triple=aarch64-linux-gnu; cross_rust_target=aarch64-unknown-linux-gnu
    cross_loader=/lib/ld-linux-aarch64.so.1 ;;
  *) echo "bootstrap-toolchain: unsupported machine $(uname -m)" >&2; exit 1 ;;
esac

# The env-var spelling cargo and the cc crate use for a per-target override:
# the Rust target triple uppercased with dashes turned into underscores.
cross_env_suffix="${cross_rust_target//-/_}"
cross_env_suffix_upper="$(echo "$cross_env_suffix" | tr '[:lower:]' '[:upper:]')"

# rustup-init is pinned by version *and* digest. The unversioned
# /rustup/dist/ URL always serves the newest release, so it cannot be pinned;
# /rustup/archive/<version>/ can. These digests are upstream's own published
# rustup-init.sha256 for 1.29.1.
case "$rust_host" in
  aarch64-unknown-linux-gnu) rustup_sha=15f6e4ce9f583b929c996c91562bad6d4454f3281de858b02cdfdef615fac433 ;;
  x86_64-unknown-linux-gnu)  rustup_sha=dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71 ;;
esac

lock="$repo_root/ci/toolchain/debs-$deb_arch.lock"

if [[ ! -f "$lock" ]]; then
  echo "bootstrap-toolchain: no lock for $deb_arch at $lock" >&2
  echo "  regenerate it with ci/toolchain/resolve-debs.py --arch $deb_arch" >&2
  exit 1
fi

# The tree is named by the lock's digest, so editing the lock selects a new
# tree instead of silently keeping the old one or rebuilding the old one in
# place. `--env-only` needs the digest too: it has to name this lock's shims.
lock_digest="$(sha256sum "$lock" | cut -d' ' -f1)"
tree="$prefix/c-toolchain/$lock_digest"
sysroot="$tree/sysroot"
shim_dir="$tree/bin"

print_env() {
  printf 'export VF_TOOLCHAIN_DIR=%q\n' "$prefix"
  printf 'export CARGO_HOME=%q\n' "$cargo_home"
  printf 'export RUSTUP_HOME=%q\n' "$rustup_home"
  # CC as well as PATH: `cc` on PATH is enough for rustc's default linker, but
  # the cc crate resolves $CC first and some build scripts read it directly.
  printf 'export CC=%q\n' "$shim_dir/cc"
  # The cross compiler has to be named explicitly, and `CC` being set is exactly
  # why. The cc crate resolves a target compiler as CC_<target>, then TARGET_CC,
  # then CC — so with only the host `CC` exported it silently uses the *host*
  # gcc to compile ring's C for the other architecture, and reports
  # `unrecognized command-line option '-m64'` (VFL-151). Naming CC_<target>
  # takes the first branch instead. AR_<target> is set for symmetry with it, and
  # so that a non-GNU archiver (llvm-ar, thin archives under LTO) can never pick
  # up the host one; GNU `ar` itself reads any ELF, so today this is insurance
  # rather than a necessity. The linker variable is cargo's own, not the cc
  # crate's, and spelled in its uppercase form.
  printf 'export CC_%s=%q\n' "$cross_env_suffix" "$shim_dir/$cross_gnu_triple-cc"
  printf 'export AR_%s=%q\n' "$cross_env_suffix" "$shim_dir/$cross_gnu_triple-ar"
  printf 'export CARGO_TARGET_%s_LINKER=%q\n' \
    "$cross_env_suffix_upper" "$shim_dir/$cross_gnu_triple-cc"
  printf 'export PATH=%q:%q:"$PATH"\n' "$shim_dir" "$cargo_home/bin"
  # Exporting PATH is not enough on an agent runner. The harness sets BASH_ENV
  # to a generated .bashrc, so *every* non-interactive bash sources it on
  # startup — and that file assigns PATH absolutely rather than appending to it.
  # The toolchain therefore resolves in the shell that evals this, and vanishes
  # in every child: `just`'s own `set shell := ["bash", "-euo", "pipefail",
  # "-c"]` makes each recipe such a child, so `just check` reported
  # `cargo: command not found` even after a successful bootstrap.
  #
  # Dropping BASH_ENV is safe here, and that is worth recording because it looks
  # like it is discarding harness setup. That file does two things. Its PATH
  # line re-asserts the harness's own entries, including the launcher directory
  # that supplies `git` and `gh`, but those are already on the PATH this env
  # inherits and prepends to, so nothing is lost. Beyond that it only turns the
  # four GIT_{AUTHOR,COMMITTER}_{NAME,EMAIL} variables from set-but-empty into
  # unset, which the loop below does itself.
  printf 'unset BASH_ENV\n'

  # The runners export several GIT_* variables with empty values. These five
  # are cleared when empty:
  #
  #   GIT_CONFIG_COUNT= makes gix — the git implementation inside cargo-audit,
  #   not the git CLI, which tolerates it — refuse to load any configuration at
  #   all. `cargo audit` then dies on `GIT_CONFIG_COUNT was not a positive
  #   integer` while fetching the advisory database, and reports it as a clone
  #   failure, which reads like a network or disk problem rather than an
  #   environment one. That failed `just audit`, and so `just check`.
  #
  #   GIT_AUTHOR_NAME= and its three companions are the ones the harness's
  #   BASH_ENV file was clearing; since that file no longer runs for our
  #   children, clear them here so a commit from a recipe keeps resolving its
  #   identity from git configuration.
  #
  # Named, not every empty GIT_*: an empty value is not always inert. The
  # runners also export GIT_ASKPASS empty, and git reads a set-but-empty
  # GIT_ASKPASS as "use no askpass helper" and skips core.askpass and
  # SSH_ASKPASS. Unsetting it would re-enable those fallbacks, so it is left
  # exactly as the harness set it. `${!var-}` reads the value without parsing
  # `env` output, so a variable is only ever unset when it really is empty.
  cat <<'GITENV'
for __vf_git_var in GIT_CONFIG_COUNT GIT_AUTHOR_NAME GIT_AUTHOR_EMAIL GIT_COMMITTER_NAME GIT_COMMITTER_EMAIL; do
  [[ -n "${!__vf_git_var-}" ]] || unset "$__vf_git_var"
done
unset __vf_git_var
GITENV
}

if [[ "${1:-}" == "--env-only" ]]; then
  # Fail rather than print a PATH to a compiler that is not there: a branch
  # whose lock differs from the one last bootstrapped has no tree yet, and
  # handing it the old lock's compiler instead would be a silent mismatch.
  if [[ ! -d "$tree" ]]; then
    echo "bootstrap-toolchain: no toolchain for this lock ($lock_digest) under $prefix" >&2
    echo "  run ci/bootstrap-toolchain.sh without --env-only to build it" >&2
    exit 1
  fi
  print_env
  exit 0
fi

mkdir -p "$prefix/c-toolchain" "$cache_dir"

# The workspace is shared between concurrent task runs, so two of them can
# reach this script at once. Serialise on a lock file rather than letting both
# extract at once; the second one finds the published tree and no-ops. This
# lock only orders bootstraps against each other — a run that is compiling
# holds nothing — which is why a published tree must never change.
exec 9>"$prefix/.bootstrap.lock"
flock 9

log() { echo "bootstrap-toolchain: $*" >&2; }

# Write one shim with a rename, never in place: bash reads a script as it runs
# it, so a concurrent `cc` must see either the old file or the new one and
# never a half-written one. Content comes from stdin.
write_shim() {
  cat > "$1.tmp.$$"
  chmod +x "$1.tmp.$$"
  mv -f "$1.tmp.$$" "$1"
}

# The same for a symlink. `ln -sf` unlinks and then creates, which leaves a
# moment where the name does not exist.
link_shim() {
  ln -sfn "$1" "$2.tmp.$$"
  mv -Tf "$2.tmp.$$" "$2"
}

# gcc relocates itself from argv[0] — $sysroot/usr/bin/../lib/gcc finds cc1 and
# collect2 — but its header and library search paths need --sysroot, and
# collect2 needs -B to find our `ld` rather than a system one that is not
# there. LD_LIBRARY_PATH is for gcc's *own* dependencies (libmpfr, libisl,
# libmpc), which also live only in the sysroot.
#
# The two -Wl flags are what keep a built binary running after this prefix
# moves or goes away. --sysroot applies to the linker too, so without them ld
# is entitled to bake $sysroot/lib/ld-linux-*.so.1 in as the ELF interpreter,
# and the binary would then depend on the toolchain directory at *runtime*.
# Both paths are deliberately the image's, not the sysroot's: the loader and
# the shared glibc are the image's copies, and the lock pins libc6 to the
# image's version precisely so that is the same glibc.
#
# Usage: write_shims <dir> <root>. The shims always name the published
# $sysroot, even when they are written into the staging directory before it is
# published; <root> is the tree to look in for which binutils exist, which is
# the staging one in that case.
write_shims() {
  local dir="$1" root="$2" alias tool real
  mkdir -p "$dir"

  write_shim "$dir/cc" <<SHIM
#!/usr/bin/env bash
# Generated by ci/bootstrap-toolchain.sh — do not edit.
export LD_LIBRARY_PATH="$sysroot/usr/lib/$gnu_triple:$sysroot/lib/$gnu_triple\${LD_LIBRARY_PATH:+:\$LD_LIBRARY_PATH}"
exec "$sysroot/usr/bin/$gnu_triple-gcc-14" \\
  --sysroot="$sysroot" \\
  -B"$sysroot/usr/bin" \\
  -B"$sysroot/usr/lib/gcc/$gnu_triple/14" \\
  -B"$sysroot/usr/libexec/gcc/$gnu_triple/14" \\
  -B"$sysroot/usr/lib/$gnu_triple" \\
  -Wl,-rpath-link,/lib/$gnu_triple \\
  -Wl,--dynamic-linker=$loader \\
  "\$@"
SHIM

  # Rust's default linker for a *-linux-gnu target is `cc`, and the `cc` crate
  # honours $CC; both names resolve to the same shim. gcc/$triple-gcc exist
  # because some build scripts look for them by name.
  for alias in gcc "$gnu_triple-gcc" "$gnu_triple-cc"; do
    link_shim cc "$dir/$alias"
  done

  # The binutils programs the `cc` crate and cargo reach for directly. These
  # need no sysroot flags, only their own shared libraries (libctf, libsframe,
  # libjansson, libzstd), so a plain exec with LD_LIBRARY_PATH is enough.
  for tool in ar ranlib nm strip objcopy objdump readelf ld ld.bfd as addr2line size strings; do
    if [[ -x "$root/usr/bin/$gnu_triple-$tool" ]]; then
      real="$sysroot/usr/bin/$gnu_triple-$tool"
    elif [[ -x "$root/usr/bin/$tool" ]]; then
      real="$sysroot/usr/bin/$tool"
    else
      continue
    fi
    write_shim "$dir/$tool" <<SHIM
#!/usr/bin/env bash
# Generated by ci/bootstrap-toolchain.sh — do not edit.
export LD_LIBRARY_PATH="$sysroot/usr/lib/$gnu_triple\${LD_LIBRARY_PATH:+:\$LD_LIBRARY_PATH}"
exec "$real" "\$@"
SHIM
    link_shim "$tool" "$dir/$gnu_triple-$tool"
  done

  # The cross compiler: the same shim idea pointed at the other architecture's
  # gcc. It is always triple-prefixed and never plain `cc`, because `cc` is the
  # host compiler and a build script that finds the wrong one there is the bug
  # this is fixing (VFL-151).
  #
  # It needs strictly less than the host shim, and the difference is worth
  # stating so nobody "restores" flags later:
  #
  #   * No -B. Debian's cross gcc keeps its private cc1, crtbegin.o and libgcc.a
  #     under /usr/lib{,exec}/gcc-cross/<triple>/14 and its `ld`/`as` under
  #     /usr/<triple>/bin, and it finds all of them by relocating from argv[0].
  #     The host shim needs -B only because the *unprefixed* names it wants are
  #     ambiguous with a system toolchain; these names are not.
  #
  #   * --sysroot is still required, for the same reason as the host: the
  #     target's libc.so is a linker script naming absolute /usr/<triple>/lib
  #     paths, and without --sysroot ld resolves them against the real root and
  #     finds nothing. The target libraries are under $sysroot/usr/<triple>,
  #     which is where this gcc looks once the sysroot is right, so the native
  #     and cross library sets coexist in one tree without colliding.
  #
  # LD_LIBRARY_PATH names the *host* triple, not the cross one: these are gcc's
  # own dependencies (libmpfr, libisl, libmpc), and the cross compiler is a host
  # binary like any other.
  #
  # --dynamic-linker is asserted rather than relied on. It already comes out as
  # the target's own unprefixed loader here, but that is a property of gcc's
  # spec file, and a sysroot-relative default would silently produce binaries
  # that depend on this prefix. The path is the *target's* loader: a
  # cross-built binary never runs on this machine, so there is no local loader
  # for it to agree with.
  #
  # The two programs print_env names are checked first, because only the lock
  # puts them here. Without the check, a lock that lost the cross set would
  # still bootstrap cleanly and the fault would surface much later as the
  # opaque `-m64` error inside `cargo check --target`.
  for tool in gcc-14 ar; do
    [[ -x "$root/usr/bin/$cross_gnu_triple-$tool" ]] && continue
    echo "bootstrap-toolchain: no $cross_gnu_triple-$tool in the sysroot for lock $lock_digest" >&2
    echo "  the lock is missing the cross set; regenerate it with ci/toolchain/resolve-debs.py --arch $deb_arch" >&2
    exit 1
  done

  write_shim "$dir/$cross_gnu_triple-cc" <<SHIM
#!/usr/bin/env bash
# Generated by ci/bootstrap-toolchain.sh — do not edit.
export LD_LIBRARY_PATH="$sysroot/usr/lib/$gnu_triple:$sysroot/lib/$gnu_triple\${LD_LIBRARY_PATH:+:\$LD_LIBRARY_PATH}"
exec "$sysroot/usr/bin/$cross_gnu_triple-gcc-14" \\
  --sysroot="$sysroot" \\
  -Wl,--dynamic-linker=$cross_loader \\
  "\$@"
SHIM
  link_shim "$cross_gnu_triple-cc" "$dir/$cross_gnu_triple-gcc"

  # The cross binutils, as scripts of their own rather than links into the host
  # set: `as` and `ld` are per-architecture programs, and the host ones cannot
  # assemble or link for the target. The rest come along so that every
  # triple-prefixed name a build script asks for is the target's own tool.
  for tool in ar ranlib nm strip objcopy objdump readelf ld ld.bfd as addr2line size strings; do
    [[ -x "$root/usr/bin/$cross_gnu_triple-$tool" ]] || continue
    write_shim "$dir/$cross_gnu_triple-$tool" <<SHIM
#!/usr/bin/env bash
# Generated by ci/bootstrap-toolchain.sh — do not edit.
export LD_LIBRARY_PATH="$sysroot/usr/lib/$gnu_triple\${LD_LIBRARY_PATH:+:\$LD_LIBRARY_PATH}"
exec "$sysroot/usr/bin/$cross_gnu_triple-$tool" "\$@"
SHIM
  done
}

if [[ -d "$tree" ]]; then
  log "toolchain already present for lock $lock_digest"
  # Regenerated on every run, each file by rename, so a prefix that has been
  # moved or copied gets shims that point at where it now is.
  write_shims "$shim_dir" "$sysroot"
else
  # Build the whole tree — sysroot, merged-/usr links and shims — in a staging
  # directory and publish it with one rename. Nothing ever names the staging
  # path, so a run killed half way leaves only a directory the next bootstrap
  # discards; and nothing can see the published tree before it is complete.
  stage="$prefix/c-toolchain/.staging-$lock_digest"
  rm -rf "$stage"
  mkdir -p "$stage/sysroot"
  log "unpacking $(grep -cv '^[[:space:]]*$' "$lock") packages for lock $lock_digest"

  while read -r name version digest url; do
    [[ -n "${name:-}" ]] || continue
    deb="$cache_dir/$(basename "$url")"
    if [[ ! -s "$deb" ]] || [[ "$(sha256sum "$deb" | cut -d' ' -f1)" != "$digest" ]]; then
      curl -fsS --retry 3 --retry-delay 2 -m 300 -L -o "$deb.part" "$url"
      mv "$deb.part" "$deb"
    fi
    got="$(sha256sum "$deb" | cut -d' ' -f1)"
    if [[ "$got" != "$digest" ]]; then
      echo "bootstrap-toolchain: digest mismatch for $name $version" >&2
      echo "  expected $digest" >&2
      echo "  got      $got" >&2
      exit 1
    fi
    dpkg-deb -x "$deb" "$stage/sysroot"
  done < "$lock"

  # Debian is merged-/usr: the packages put everything under /usr and the base
  # filesystem supplies /lib -> usr/lib. `dpkg-deb -x` unpacks package contents
  # only, so a bare extraction has no such symlink, and the absolute paths in
  # libc6-dev's `libc.so` linker script — /lib/<triple>/libc.so.6 and
  # /lib/ld-linux-*.so.1 — then resolve to nothing inside --sysroot. Recreate
  # the base layout's links so the sysroot looks like a real root. lib64 is
  # amd64's: its loader is /lib64/ld-linux-x86-64.so.2. The links are relative,
  # so they survive the rename below.
  for merged in lib lib64 bin sbin; do
    [[ -d "$stage/sysroot/usr/$merged" ]] || continue
    ln -sfn "usr/$merged" "$stage/sysroot/$merged"
  done

  write_shims "$stage/bin" "$stage/sysroot"
  mv -T "$stage" "$tree"
  log "toolchain ready for lock $lock_digest"
fi

# Rust. rustup reads rust-toolchain.toml, so the channel, components and target
# set come from the committed pin (§A5) and are not restated here.
export CARGO_HOME="$cargo_home"
export RUSTUP_HOME="$rustup_home"
# Exported before rustup-init runs, not after: rustup-init probes for a default
# linker on startup and warns that no C toolchain is present if the shims are
# not already on PATH. The warning is harmless but it is the exact message this
# script exists to eliminate, so it must not appear in the bootstrap's own log.
export PATH="$shim_dir:$cargo_home/bin:$PATH"

if [[ -x "$cargo_home/bin/rustup" ]]; then
  log "rustup already present ($("$cargo_home/bin/rustup" --version 2>/dev/null | head -1))"
else
  log "installing rustup $RUSTUP_VERSION"
  init="$cache_dir/rustup-init-$RUSTUP_VERSION-$rust_host"
  if [[ ! -s "$init" ]] || [[ "$(sha256sum "$init" | cut -d' ' -f1)" != "$rustup_sha" ]]; then
    curl -fsS --retry 3 --retry-delay 2 -m 300 -L -o "$init.part" \
      "https://static.rust-lang.org/rustup/archive/$RUSTUP_VERSION/$rust_host/rustup-init"
    mv "$init.part" "$init"
  fi
  got="$(sha256sum "$init" | cut -d' ' -f1)"
  if [[ "$got" != "$rustup_sha" ]]; then
    echo "bootstrap-toolchain: rustup-init digest mismatch" >&2
    echo "  expected $rustup_sha" >&2
    echo "  got      $got" >&2
    exit 1
  fi
  chmod +x "$init"
  # --no-modify-path: the env comes from print_env below, not from a dotfile a
  # runner would not source anyway.
  "$init" -y --no-modify-path --default-toolchain none --profile minimal >&2
fi

# Materialise the pinned toolchain now rather than on whichever cargo command
# a coder happens to run first, so the multi-minute download is attributed to
# the bootstrap and not to a test run that looks like it hung.
if [[ -f "$repo_root/rust-toolchain.toml" ]]; then
  log "installing the toolchain pinned in rust-toolchain.toml"
  ( cd "$repo_root" && rustup show active-toolchain >&2 )
fi

# Both compilers are reported so the log says which pair this lock produced.
# This line is a report, not a check: a command substitution that fails inside
# an argument does not trip `set -e`. The check is the one in write_shims.
log "ready — $(cd "$repo_root" && rustc --version 2>&1), $("$shim_dir/cc" --version | head -1), cross $("$shim_dir/$cross_gnu_triple-cc" --version | head -1)"
print_env
