#!/usr/bin/env python3
"""resolve-debs.py — regenerate a debs-<arch>.lock for ci/bootstrap-toolchain.sh.

The lock pins the exact Debian packages that make up a non-root C toolchain:
name, version, SHA256 and archive URL, one package per line. It is generated,
not hand-edited — a hand-edited lock drifts from the dependency closure and the
first thing anyone notices is a link error three crates deep.

This runs when the toolchain is deliberately moved (a new Debian suite, a new
gcc major, a new architecture), not on every bootstrap. The bootstrap only ever
reads the committed lock, so a normal run touches no package index and resolves
no dependency: it downloads exactly what was reviewed.

Two choices are worth recording:

  * Only packaging machinery is excluded (SKIP). Every shared library stays in
    the closure, because the extracted tree has to be a self-contained sysroot
    — ld resolves the absolute paths inside libc6-dev's `libc.so` linker script
    relative to --sysroot, so libc6 must be inside it.

  * An `a | b` dependency takes `a`. The closure here is a C toolchain, where
    the alternatives are things like `debconf | debconf-2.0`; both land in SKIP
    and nothing in the real closure depends on picking the second branch.

Usage:
  ci/toolchain/resolve-debs.py --arch arm64 > ci/toolchain/debs-arm64.lock

Review the diff before committing it: this is the supply chain for every build
on every runner, and the digests are the only thing standing between a
bootstrap and an unreviewed binary.
"""

from __future__ import annotations

import argparse
import gzip
import re
import sys
import tempfile
import urllib.request
from collections import OrderedDict

SUITE = "trixie"

# Where each suite's index lives. trixie-updates and trixie-security are
# included because a glibc point update lands there first; both are allowed to
# be absent, which is the normal state shortly after a release.
SOURCES = [
    ("https://deb.debian.org/debian/", f"dists/{SUITE}/main/binary-{{arch}}/Packages.gz", True),
    ("https://deb.debian.org/debian/", f"dists/{SUITE}-updates/main/binary-{{arch}}/Packages.gz", False),
    ("https://deb.debian.org/debian-security/", f"dists/{SUITE}-security/main/binary-{{arch}}/Packages.gz", False),
]

# The toolchain we actually want. Everything else in the lock is here because
# one of these four pulls it in.
SEEDS = ["gcc-14", "libc6-dev", "binutils", "libgcc-14-dev"]

# Packaging and scripting machinery: needed to *install* a package, irrelevant
# to unpacking one and compiling C with it.
SKIP = {
    "debconf", "debconf-2.0", "dpkg", "perl", "perl-base", "libperl5.40",
    "install-info", "libdebconfclient0", "sensible-utils", "ucf",
    "dpkg-dev", "gcc-14-doc", "binutils-doc",
}


def fetch_index(base: str, path: str, required: bool) -> str | None:
    url = base + path
    try:
        with urllib.request.urlopen(url, timeout=180) as resp:
            raw = resp.read()
    except Exception as exc:
        if required:
            raise SystemExit(f"resolve-debs: cannot fetch {url}: {exc}")
        print(f"# optional index unavailable: {url} ({exc})", file=sys.stderr)
        return None
    try:
        return gzip.decompress(raw).decode("utf-8", "replace")
    except OSError:
        # The archive serves an HTML 404 for a suite that has no index yet.
        if required:
            raise SystemExit(f"resolve-debs: {url} is not a gzip index")
        print(f"# optional index absent (not gzip): {url}", file=sys.stderr)
        return None


def parse(blob: str, base: str) -> dict[str, dict]:
    pkgs: dict[str, dict] = {}
    for stanza in blob.split("\n\n"):
        if not stanza.strip():
            continue
        fields: dict[str, str] = {}
        key = None
        for line in stanza.split("\n"):
            if line.startswith((" ", "\t")):
                if key:
                    fields[key] += "\n" + line.strip()
                continue
            if ":" not in line:
                continue
            key, _, val = line.partition(":")
            key = key.strip()
            fields[key] = val.strip()
        name = fields.get("Package")
        if not name:
            continue
        fields["_base"] = base
        prev = pkgs.get(name)
        # Lexicographic, not dpkg version order. Good enough here: the indexes
        # are disjoint per package in practice, and the lock is reviewed by
        # hand before it is committed.
        if prev is None or fields.get("Version", "") > prev.get("Version", ""):
            pkgs[name] = fields
    return pkgs


def dep_names(field: str) -> list[str]:
    names = []
    for clause in field.split(","):
        clause = clause.strip()
        if not clause:
            continue
        first = clause.split("|")[0].strip()
        match = re.match(r"^([a-z0-9][a-z0-9+.\-]*)", first)
        if match:
            names.append(match.group(1))
    return names


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--arch", required=True, choices=["arm64", "amd64"],
                    help="Debian architecture name")
    args = ap.parse_args()

    index: dict[str, dict] = {}
    for base, path, required in SOURCES:
        blob = fetch_index(base, path.format(arch=args.arch), required)
        if blob:
            index.update(parse(blob, base))
    print(f"# indexed {len(index)} packages for {args.arch}", file=sys.stderr)

    # Virtual packages, so a dependency on e.g. `libc-dev` finds libc6-dev.
    provides: dict[str, str] = {}
    for name, fields in index.items():
        for clause in fields.get("Provides", "").split(","):
            clause = clause.strip()
            if clause:
                provides.setdefault(clause.split()[0], name)

    resolved: OrderedDict[str, dict] = OrderedDict()
    queue = list(SEEDS)
    missing: list[str] = []
    while queue:
        name = queue.pop(0)
        if name in resolved or name in SKIP:
            continue
        fields = index.get(name)
        if fields is None:
            real = provides.get(name)
            if real and real not in resolved:
                queue.append(real)
                continue
            missing.append(name)
            continue
        resolved[name] = fields
        for field in ("Pre-Depends", "Depends"):
            queue.extend(d for d in dep_names(fields.get(field, ""))
                         if d not in resolved and d not in SKIP)

    if missing:
        raise SystemExit(f"resolve-debs: unresolved dependencies: {sorted(set(missing))}")

    total = 0
    out = []
    for name, fields in sorted(resolved.items()):
        digest = fields.get("SHA256")
        if not digest:
            raise SystemExit(f"resolve-debs: {name} has no SHA256 in the index")
        total += int(fields.get("Size", 0))
        out.append(f"{name} {fields['Version']} {digest} {fields['_base']}{fields['Filename']}")
        print(f"# {name:34} {fields['Version']}", file=sys.stderr)

    print(f"# {len(resolved)} packages, {total // 1024 // 1024} MiB", file=sys.stderr)
    print("\n".join(out))
    return 0


if __name__ == "__main__":
    sys.exit(main())
