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

Four choices are worth recording:

  * Every index is authenticated before it is read. Each suite's InRelease is
    verified with sqv against the Debian archive keyring and must name its
    own suite and be inside its Valid-Until, and a Packages file is parsed
    only if its SHA256 and size match what the signed text gives for it. HTTPS proves which mirror answered, not that the archive signed
    what it sent, and the lock's digests are only as good as the index they
    came from.

  * Only packaging machinery is excluded (SKIP). Every shared library stays in
    the closure, because the extracted tree has to be a self-contained sysroot
    — ld resolves the absolute paths inside libc6-dev's `libc.so` linker script
    relative to --sysroot, so libc6 must be inside it.

  * An `a | b` dependency takes `a`. The closure here is a C toolchain, where
    the alternatives are things like `debconf | debconf-2.0`; both land in SKIP
    and nothing in the real closure depends on picking the second branch.

  * When a package appears more than once — within one index, or in more than
    one of SOURCES — the highest version wins, by dpkg's own ordering. Plain
    string order gets this wrong, and did: main lists linux-libc-dev at both
    6.12.94-1 and 6.12.107-1, and string order kept 6.12.94-1 because "9" >
    "1". "Last index wins" is no better; it silently downgrades the day a
    later source carries an older build. On an exact tie the later source
    wins, so a package published in both main and -security is fetched from
    -security.

Usage:
  ci/toolchain/resolve-debs.py --arch arm64 > ci/toolchain/debs-arm64.lock

It needs sqv and debian-archive-keyring, which apt depends on in trixie.

Review the diff before committing it: this is the supply chain for every build
on every runner, and the digests are the only thing standing between a
bootstrap and an unreviewed binary. Check libc6 and libc6-dev in particular:
they must still equal the runner image's glibc (`dpkg-query -W libc6`), and a
glibc update in -security moves them ahead of an image that has not had it.
"""

from __future__ import annotations

import argparse
import datetime
import email.utils
import gzip
import hashlib
import http.client
import lzma
import os
import re
import shutil
import ssl
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from collections import OrderedDict

SUITE = "trixie"

# Fetch retries, matching the bootstrap's `curl --retry 3 --retry-delay 2`, so
# one transient error does not abort a regeneration.
RETRIES = 3
RETRY_DELAY = 2
TRANSIENT_HTTP = {408, 429, 500, 502, 503, 504}

DIGITS = "0123456789"
LETTERS = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz"

# Where each suite lives. trixie-updates and trixie-security are included
# because a point update — glibc, the kernel headers — lands there first. Every
# one is required: a stable suite publishes all three from release day, and an
# index that is quietly skipped is a security update that quietly never
# reaches the lock.
SOURCES = [
    ("https://deb.debian.org/debian/", SUITE),
    ("https://deb.debian.org/debian/", f"{SUITE}-updates"),
    ("https://deb.debian.org/debian-security/", f"{SUITE}-security"),
]

# The keys that sign every suite's InRelease. On trixie, apt depends on both
# debian-archive-keyring and sqv, so any image with apt has what this needs.
KEYRING = "/usr/share/keyrings/debian-archive-keyring.gpg"

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


def fetch(url: str) -> bytes | None:
    """Return the body at `url`, or None if the server answers 404.

    Retries like the bootstrap's `curl --retry 3 --retry-delay 2`: a timeout,
    a dropped connection or a 408/429/5xx is tried again, up to RETRIES times,
    RETRY_DELAY seconds apart. Unlike curl without --retry-connrefused, a
    refused connection is retried too. Any other HTTP status is an answer,
    not a hiccup, and a certificate that fails to verify is never retried.
    """
    err: Exception | None = None
    for attempt in range(RETRIES + 1):
        if attempt:
            print(f"# retry {attempt}/{RETRIES} {url}: {err}", file=sys.stderr)
            time.sleep(RETRY_DELAY)
        try:
            with urllib.request.urlopen(url, timeout=180) as resp:
                return resp.read()
        except urllib.error.HTTPError as exc:
            if exc.code == 404:
                return None
            if exc.code not in TRANSIENT_HTTP:
                raise SystemExit(f"resolve-debs: cannot fetch {url}: {exc}")
            err = exc
        except urllib.error.URLError as exc:
            if isinstance(exc.reason, ssl.SSLCertVerificationError):
                raise SystemExit(f"resolve-debs: cannot fetch {url}: {exc}")
            err = exc
        except (OSError, http.client.HTTPException) as exc:
            err = exc
    raise SystemExit(f"resolve-debs: cannot fetch {url} after {RETRIES} retries: {err}")


def read_release(dist: str, suite: str, keyring: str) -> dict[str, str]:
    """Fetch `dist`InRelease, verify its signature, and return its fields.

    Only the text sqv writes out is parsed, never the download itself, so
    nothing outside the signature can reach a digest. A good signature alone
    is not enough, so the Codename and Valid-Until checks follow apt: a mirror
    could serve another suite's genuine InRelease, or replay a stale one from
    before a security fix.
    """
    url = dist + "InRelease"
    signed = fetch(url)
    if signed is None:
        raise SystemExit(f"resolve-debs: no InRelease at {url}")
    sqv = shutil.which("sqv")
    if sqv is None:
        raise SystemExit("resolve-debs: sqv is needed to verify InRelease (apt install sqv)")
    if not os.path.isfile(keyring):
        raise SystemExit(f"resolve-debs: no keyring at {keyring} "
                         "(apt install debian-archive-keyring, or pass --keyring)")
    with tempfile.TemporaryDirectory() as tmp:
        src, out = os.path.join(tmp, "InRelease"), os.path.join(tmp, "Release")
        with open(src, "wb") as fh:
            fh.write(signed)
        proc = subprocess.run([sqv, "--keyring", keyring, "--cleartext", "--output", out, src],
                              capture_output=True, text=True)
        if proc.returncode != 0:
            raise SystemExit(f"resolve-debs: {url} does not verify against {keyring}: "
                             f"{proc.stderr.strip()}")
        with open(out, encoding="utf-8") as fh:
            release = stanza_fields(fh.read())
    if release.get("Codename") != suite:
        raise SystemExit(f"resolve-debs: {url} is for {release.get('Codename')!r}, not {suite!r}")
    if "Valid-Until" in release:
        try:
            until = email.utils.parsedate_to_datetime(release["Valid-Until"])
        except (TypeError, ValueError) as exc:
            raise SystemExit(f"resolve-debs: {url} has a bad Valid-Until: {exc}")
        if until.tzinfo is None or until <= datetime.datetime.now(datetime.timezone.utc):
            raise SystemExit(f"resolve-debs: {url} expired at {release['Valid-Until']}")
    print(f"# verified {url}, signed by {' '.join(proc.stdout.split())}", file=sys.stderr)
    return release


def fetch_index(base: str, suite: str, arch: str, keyring: str) -> str:
    # .xz first. -updates and -security publish *only* Packages.xz, so a reader
    # that asked for .gz got a 404 from both, took it for an empty suite, and
    # never read -security at all. .gz stays as the fallback for a mirror that
    # carries only that. The signed InRelease says which of the two exist, so
    # nothing is probed, and its SHA256 and size are checked before the index
    # is decompressed.
    dist = f"{base}dists/{suite}/"
    release = read_release(dist, suite, keyring)
    sums: dict[str, tuple[int, str]] = {}
    for line in release.get("SHA256", "").split("\n"):
        parts = line.split()
        if len(parts) == 3:
            sums[parts[2]] = (int(parts[1]), parts[0].lower())
    for suffix, decompress in ((".xz", lzma.decompress), (".gz", gzip.decompress)):
        path = f"main/binary-{arch}/Packages{suffix}"
        if path not in sums:
            continue
        size, digest = sums[path]
        # By hash where the archive offers it, as apt does, so the file is the
        # one this InRelease names even if the mirror updates in between.
        if release.get("Acquire-By-Hash") == "yes":
            url = f"{dist}main/binary-{arch}/by-hash/SHA256/{digest}"
        else:
            url = dist + path
        raw = fetch(url)
        if raw is None:
            raise SystemExit(f"resolve-debs: {url} is listed in {dist}InRelease but missing")
        got = hashlib.sha256(raw).hexdigest()
        if len(raw) != size or got != digest:
            raise SystemExit(f"resolve-debs: {url} does not match {dist}InRelease: "
                             f"got {got} ({len(raw)} bytes), want {digest} ({size} bytes)")
        try:
            blob = decompress(raw).decode("utf-8", "replace")
        except (OSError, lzma.LZMAError) as exc:
            raise SystemExit(f"resolve-debs: {url} is not a valid {suffix} index: {exc}")
        print(f"# read {url}", file=sys.stderr)
        return blob
    raise SystemExit(f"resolve-debs: {dist}InRelease has no SHA256 for "
                     f"main/binary-{arch}/Packages.xz or .gz")


def stanza_fields(stanza: str) -> dict[str, str]:
    """Split one deb822 stanza into fields; continuation lines join with \\n."""
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
    return fields


def parse(blob: str, base: str) -> dict[str, dict]:
    pkgs: dict[str, dict] = {}
    for stanza in blob.split("\n\n"):
        if not stanza.strip():
            continue
        fields = stanza_fields(stanza)
        name = fields.get("Package")
        if not name:
            continue
        fields["_base"] = base
        offer(pkgs, name, fields)
    return pkgs


def _order(c: str) -> int:
    # dpkg's lib/dpkg/version.c `order()`: within a non-digit run, `~` sorts
    # before everything, even the end of the string; letters sort before
    # other characters.
    if c == "~":
        return -1
    if c in DIGITS:
        return 0
    if c in LETTERS:
        return ord(c)
    return ord(c) + 256


def _verrevcmp(a: str, b: str) -> int:
    # dpkg's `verrevcmp()`: alternate non-digit runs, compared by _order, with
    # digit runs, compared as numbers. "" stands for the end of the string,
    # which _order sees as 0.
    i = j = 0
    while i < len(a) or j < len(b):
        while (i < len(a) and a[i] not in DIGITS) or (j < len(b) and b[j] not in DIGITS):
            ac = _order(a[i]) if i < len(a) else 0
            bc = _order(b[j]) if j < len(b) else 0
            if ac != bc:
                return ac - bc
            i += 1
            j += 1
        while i < len(a) and a[i] == "0":
            i += 1
        while j < len(b) and b[j] == "0":
            j += 1
        first_diff = 0
        while i < len(a) and a[i] in DIGITS and j < len(b) and b[j] in DIGITS:
            if not first_diff:
                first_diff = ord(a[i]) - ord(b[j])
            i += 1
            j += 1
        if i < len(a) and a[i] in DIGITS:
            return 1
        if j < len(b) and b[j] in DIGITS:
            return -1
        if first_diff:
            return first_diff
    return 0


def compare_versions(a: str, b: str) -> int:
    """Order two Debian versions as `dpkg --compare-versions` does."""
    def split(v: str) -> tuple[int, str, str]:
        epoch, colon, rest = v.partition(":")
        if not colon:
            epoch, rest = "0", v
        upstream, hyphen, revision = rest.rpartition("-")
        if not hyphen:
            upstream, revision = rest, ""
        return int(epoch), upstream, revision

    ea, ua, ra = split(a)
    eb, ub, rb = split(b)
    if ea != eb:
        return ea - eb
    return _verrevcmp(ua, ub) or _verrevcmp(ra, rb)


def offer(index: dict[str, dict], name: str, fields: dict) -> None:
    """Keep `fields` for `name` unless `index` already holds a newer version."""
    prev = index.get(name)
    if prev is None or compare_versions(fields.get("Version", ""), prev.get("Version", "")) >= 0:
        index[name] = fields


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
    ap.add_argument("--keyring", default=KEYRING,
                    help="keyring that signs the suites' InRelease (default: %(default)s)")
    args = ap.parse_args()

    index: dict[str, dict] = {}
    for base, suite in SOURCES:
        blob = fetch_index(base, suite, args.arch, args.keyring)
        for name, fields in parse(blob, base).items():
            offer(index, name, fields)
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
            # A virtual name is satisfied by its provider, whether that is
            # already resolved or still to come; only a name nothing provides
            # is missing.
            real = provides.get(name)
            if real is None:
                missing.append(name)
            elif real not in resolved:
                queue.append(real)
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
