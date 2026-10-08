#!/usr/bin/env bash
# Local harness for ci/lane-gate.sh. Builds a tiny fixture repo and asserts each
# subcommand's verdict on a set of representative diffs.
set -uo pipefail

# The Paperclip GitHub launcher blanks the git identity vars and shadows `git`
# on PATH; both have to go for a local fixture repo to work.
unset GIT_AUTHOR_NAME GIT_AUTHOR_EMAIL GIT_COMMITTER_NAME GIT_COMMITTER_EMAIL
unset GIT_CONFIG_GLOBAL GIT_CONFIG_SYSTEM GIT_CONFIG_COUNT
export PATH="/usr/bin:/bin:/usr/local/bin"
export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@example.com
export GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@example.com

# Resolved before the harness cd's into its fixture, so a relative argument works.
GATE="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
W="$(mktemp -d)"
trap 'rm -rf "$W"' EXIT
cd "$W" || exit 1

pass_count=0; fail_count=0

setup() {
  rm -rf "$W/r"; mkdir -p "$W/r"; cd "$W/r" || exit 1
  git init -q -b main .
  mkdir -p ci crates/vf-core/src crates/vf-core/tests fuzz
  cp "$GATE" ci/lane-gate.sh; chmod +x ci/lane-gate.sh
  printf 'pub fn add(a:i32,b:i32)->i32{a+b}\n' > crates/vf-core/src/lib.rs
  printf '#[test]\nfn adds(){assert_eq!(vf_core::add(1,2),3);}\n' > crates/vf-core/tests/add.rs
  printf '#[test]\nfn two(){assert!(true);}\n#[test]\nfn three(){assert!(true);}\n' > crates/vf-core/tests/more.rs
  printf '# docs\n' > README.md
  git add -A >/dev/null; git commit -qm base
  git branch baseline
}

# check <name> <subcommand> <expected: pass|fail>
check() {
  local name="$1" sub="$2" expect="$3" out rc
  out="$(./ci/lane-gate.sh "$sub" baseline HEAD 2>&1)"; rc=$?
  local got=pass; [ $rc -ne 0 ] && got=fail
  if [ "$got" = "$expect" ]; then
    printf 'ok    %-46s %s=%s\n' "$name" "$sub" "$got"; pass_count=$((pass_count+1))
  else
    printf 'NOT OK %-45s %s: expected %s got %s\n' "$name" "$sub" "$expect" "$got"
    printf '%s\n' "$out" | sed 's/^/        | /'
    fail_count=$((fail_count+1))
  fi
}

commit() { git add -A >/dev/null; git commit -qm "$1"; }

# --- 1. production-only change: allowed -------------------------------------
setup
printf 'pub fn add(a:i32,b:i32)->i32{a+b}\npub fn sub(a:i32,b:i32)->i32{a-b}\n' > crates/vf-core/src/lib.rs
commit prod
check "prod-only change" partition pass
check "prod-only change" erosion pass
check "prod-only change" inline-tests pass

# --- 2. test-only change: allowed ------------------------------------------
setup
printf '#[test]\nfn subs(){assert_eq!(vf_core::sub(3,1),2);}\n' > crates/vf-core/tests/sub.rs
commit tests
check "test-only change" partition pass
check "test-only change" erosion pass

# --- 3. the lane crossing: refused ----------------------------------------
setup
printf 'pub fn add(a:i32,b:i32)->i32{a+b}\npub fn sub(a:i32,b:i32)->i32{a-b}\n' > crates/vf-core/src/lib.rs
printf '#[test]\nfn subs(){assert_eq!(vf_core::sub(3,1),2);}\n' > crates/vf-core/tests/sub.rs
commit mixed
check "prod + test in one PR" partition fail

# --- 4. neutral files may accompany either lane ---------------------------
setup
printf 'pub fn add(a:i32,b:i32)->i32{a+b}\npub fn sub(a:i32,b:i32)->i32{a-b}\n' > crates/vf-core/src/lib.rs
printf '# docs\nmore\n' > README.md
commit prod-plus-docs
check "prod + README" partition pass

# --- 5. gate change bundled with code: refused ---------------------------
setup
printf '\n# tweak\n' >> ci/lane-gate.sh
printf 'pub fn add(a:i32,b:i32)->i32{a+b}\npub fn sub(a:i32,b:i32)->i32{a-b}\n' > crates/vf-core/src/lib.rs
commit gate-plus-code
check "gate change + prod" partition fail

# --- 6. gate change alone: allowed ---------------------------------------
setup
printf '\n# tweak\n' >> ci/lane-gate.sh
commit gate-only
check "gate change alone" partition pass

# --- 7. #[ignore] added: refused -----------------------------------------
setup
printf '#[test]\n#[ignore]\nfn adds(){assert_eq!(vf_core::add(1,2),3);}\n' > crates/vf-core/tests/add.rs
commit ignore
check "adds #[ignore]" erosion fail

# --- 8. test deleted: refused --------------------------------------------
setup
rm crates/vf-core/tests/more.rs
commit delete-test
check "deletes a test file" erosion fail

# --- 9. single #[test] removed from a kept file: refused -----------------
setup
printf '#[test]\nfn two(){assert!(true);}\n' > crates/vf-core/tests/more.rs
commit drop-one-test
check "removes one #[test]" erosion fail

# --- 10. test moved between files: allowed (count preserved) -------------
setup
git mv crates/vf-core/tests/more.rs crates/vf-core/tests/renamed.rs
commit move-test
check "moves a test file" erosion pass

# --- 11. inline #[cfg(test)] module: refused ----------------------------
setup
printf 'pub fn add(a:i32,b:i32)->i32{a+b}\n#[cfg(test)]\nmod tests{#[test] fn t(){}}\n' > crates/vf-core/src/lib.rs
commit inline
check "inline #[cfg(test)] in src" inline-tests fail

# --- 12. fuzz target counts as a test ----------------------------------
setup
printf 'fuzz_target!(|d:&[u8]|{});\n' > fuzz/a.rs
commit add-fuzz
check "adds a fuzz target" erosion pass
setup
printf 'fuzz_target!(|d:&[u8]|{});\n' > fuzz/a.rs
commit add-fuzz
rm fuzz/a.rs; commit rm-fuzz
check "removes a fuzz target" erosion pass   # vs baseline count is unchanged

# --- 13. a tree with no Rust at all ------------------------------------
# Regression: git grep exits non-zero when it matches nothing, which under
# `set -e -o pipefail` aborted `erosion` with no verdict. This is the ordinary
# state of a repository before any code lands, and it must simply pass.
setup_bare() {
  rm -rf "$W/r"; mkdir -p "$W/r"; cd "$W/r" || exit 1
  git init -q -b main .
  mkdir -p ci
  cp "$GATE" ci/lane-gate.sh; chmod +x ci/lane-gate.sh
  printf '# repo\n' > README.md
  git add -A >/dev/null; git commit -qm base
  git branch baseline
}
setup_bare
printf '# repo\nmore\n' > README.md
commit docs-only
check "no .rs anywhere in the tree" erosion pass
check "no .rs anywhere in the tree" partition pass
check "no .rs anywhere in the tree" inline-tests pass

# --- 14. first Rust test added to an empty tree ------------------------
setup_bare
mkdir -p crates/vf-core/tests
printf '#[test]\nfn first(){assert!(true);}\n' > crates/vf-core/tests/first.rs
commit first-test
check "first test in a previously bare tree" erosion pass

# --- 15. empty diff -----------------------------------------------------
setup
check "empty diff" partition pass

# --- 16. pins: a commit SHA with a trailing tag comment: allowed ---------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262 # v4.4.0\n' > .github/workflows/ci.yml
commit add-workflow-pinned
check "uses: pinned to a SHA with a tag comment" pins pass

# --- 17. pins: a tag instead of a SHA: refused ----------------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@v4\n' > .github/workflows/ci.yml
commit add-workflow-tag
check "uses: pinned to a tag (@v4)" pins fail

# --- 18. pins: a branch instead of a SHA: refused -------------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@main\n' > .github/workflows/ci.yml
commit add-workflow-branch
check "uses: pinned to a branch (@main)" pins fail

# --- 19. pins: a 39-character SHA (too short): refused --------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af67726\n' > .github/workflows/ci.yml
commit add-workflow-short-sha
check "uses: SHA is 39 hex characters" pins fail

# --- 20. pins: a 41-character SHA (too long): refused ----------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af6772622\n' > .github/workflows/ci.yml
commit add-workflow-long-sha
check "uses: SHA is 41 hex characters" pins fail

# --- 21. pins: an uppercase SHA: refused -----------------------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11D5960A326750D5838078E36CF38B85AF677262\n' > .github/workflows/ci.yml
commit add-workflow-upper-sha
check "uses: SHA is uppercase hex" pins fail

# --- 22. pins: no @ref at all: refused -------------------------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout\n' > .github/workflows/ci.yml
commit add-workflow-no-ref
check "uses: has no @ref" pins fail

# --- 23. pins: a local ./ action: refused -----------------------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: ./.github/actions/my-action\n' > .github/workflows/ci.yml
commit add-workflow-local-action
check "uses: a local ./ action" pins fail

# --- 24. pins: a docker:// image: refused -----------------------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: docker://alpine:3.18\n' > .github/workflows/ci.yml
commit add-workflow-docker-image
check "uses: a docker:// image" pins fail

# --- 25. pins: an empty value: refused --------------------------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses:\n' > .github/workflows/ci.yml
commit add-workflow-empty-value
check "uses: has an empty value" pins fail

# --- 26. pins: a block-scalar value: refused --------------------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: >-\n          actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' > .github/workflows/ci.yml
commit add-workflow-block-scalar
check "uses: a block-scalar (>-) value" pins fail

# --- 27. pins: a commented-out uses: line is ignored: allowed --------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      # - uses: actions/checkout@v4\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' > .github/workflows/ci.yml
commit add-workflow-commented-and-pinned
check "commented-out uses: line is ignored" pins pass

# --- 28. pins: a quoted key/value in a flow mapping: allowed ---------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - { "uses": "actions/checkout@11d5960a326750d5838078e36cf38b85af677262" }\n' > .github/workflows/ci.yml
commit add-workflow-quoted-flow-mapping
check "quoted uses: in a flow mapping" pins pass

# --- 29. pins: a workflow with no uses: key at all: allowed ----------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - run: echo hi\n' > .github/workflows/ci.yml
commit add-workflow-no-uses-key
check "workflow with no uses: key" pins pass

# --- 30. pins: a tree with no .github/workflows/ at all: allowed -----------
# Regression: setup() creates no .github/workflows/ today, so this is the
# ordinary state before any workflow lands, and git grep's no-match exit (1)
# must not be mistaken for a read failure.
setup
check "no .github/workflows/ anywhere in the tree" pins pass

# --- 31. all: an unpinned uses: fails the suite even though the other -----
#          three checks on this diff would pass on their own ---------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@v4\n' > .github/workflows/ci.yml
commit add-workflow-tag-via-all
check "all: unpinned uses: fails the combined run" all fail

# --- 32. pins: an unresolvable head-ref: refused ----------------------------
# check() always compares baseline against HEAD, so this case calls the gate
# directly with a ref that cannot be resolved.
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' > .github/workflows/ci.yml
commit add-workflow-for-bad-head-ref
out="$(./ci/lane-gate.sh pins baseline does-not-exist-ref 2>&1)"; rc=$?
name="unresolvable head-ref"
if [ $rc -ne 0 ]; then
  printf 'ok    %-46s %s=%s\n' "$name" pins fail; pass_count=$((pass_count+1))
else
  printf 'NOT OK %-45s %s: expected fail got pass\n' "$name" pins
  printf '%s\n' "$out" | sed 's/^/        | /'
  fail_count=$((fail_count+1))
fi

#   The cases below (33-48) are regression fixtures for VFL-122 review M1
#   (e0be0ce): `pins` used to test only the leftmost uses: on a line, so a
#   pinned decoy earlier on the line let an unpinned ref after it pass.

# --- 33. pins: a pinned decoy before an unpinned uses:, same line: refused --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps: [{uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262}, {uses: evil/act@v4}]\n' > .github/workflows/ci.yml
commit add-workflow-pinned-decoy-first
check "pinned decoy before an unpinned uses: (same line)" pins fail

# --- 34. pins: same decoy, reversed order: refused --------------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps: [{uses: evil/act@v4}, {uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262}]\n' > .github/workflows/ci.yml
commit add-workflow-pinned-decoy-second
check "pinned decoy after an unpinned uses: (same line)" pins fail

# --- 35. pins: a uses: inside a quoted string masks the real key: refused ---
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - { name: "replaces uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262", uses: actions/checkout@v4 }\n' > .github/workflows/ci.yml
commit add-workflow-uses-inside-string
check "uses: text inside a quoted string, real key unpinned" pins fail

# --- 36. pins: a plain value cut short by a comma (@<sha>,v4): refused ------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: evil/act@11d5960a326750d5838078e36cf38b85af677262,v4\n' > .github/workflows/ci.yml
commit add-workflow-comma-suffix
check "uses: value followed by ,v4 with no whitespace" pins fail

# --- 37. pins: two valid uses: values on one line: allowed ------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps: [{ uses: a/b@11d5960a326750d5838078e36cf38b85af677262 }, { uses: "c/d@11d5960a326750d5838078e36cf38b85af677262" }]\n' > .github/workflows/ci.yml
commit add-workflow-two-pinned-values
check "two valid pinned uses: values on one line" pins pass

# --- 38. pins: uses: inside a flow-sequence bracket: refused ----------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps: [uses: x/y@v4]\n' > .github/workflows/ci.yml
commit add-workflow-bracket-key
check "uses: key opening a flow-sequence bracket" pins fail

# --- 39. pins: a !!str tag before the uses: key: refused --------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - !!str uses: x/y@v4\n' > .github/workflows/ci.yml
commit add-workflow-tagged-key
check "!!str tag before an unpinned uses: key" pins fail

# --- 40. pins: an anchor (&k) before the uses: key: refused -----------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - &k uses: x/y@v4\n' > .github/workflows/ci.yml
commit add-workflow-anchored-key
check "anchor (&k) before an unpinned uses: key" pins fail

# --- 41. pins: an explicit "? uses" key, value on the next line: refused ----
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - ? uses\n        : x/y@v4\n' > .github/workflows/ci.yml
commit add-workflow-explicit-key
check "explicit ? uses key with value on the next line" pins fail

# --- 42. pins: a single-quoted value with a doubled '': refused ------------
# The doubled '' is YAML's escape for a literal quote inside a single-quoted
# scalar. SQ_VALUE_RE requires the character after the closing quote to be a
# non-quote (or end of line), so it does not match here; the scanner falls
# back to PLAIN_VALUE_RE, which cannot produce a value that satisfies PIN_RE.
# This refuses a line it cannot parse rather than guessing at its value.
setup
mkdir -p .github/workflows
printf "on: push\njobs:\n  build:\n    steps:\n      - uses: 'x/y@11d5960a326750d5838078e36cf38b85af677262''v4'\n" > .github/workflows/ci.yml
commit add-workflow-doubled-single-quote
check "single-quoted value with a doubled '' escape" pins fail

# --- 43. pins: a local ./ action pinned to a SHA: refused -------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: ./.github/actions/x@11d5960a326750d5838078e36cf38b85af677262\n' > .github/workflows/ci.yml
commit add-workflow-local-action-pinned
check "local ./ action pinned to a SHA" pins fail

# --- 44. pins: a docker:// image pinned to a SHA: refused -------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: docker://alpine@11d5960a326750d5838078e36cf38b85af677262\n' > .github/workflows/ci.yml
commit add-workflow-docker-image-pinned
check "docker:// image pinned to a SHA" pins fail

# --- 45. pins: a comment line holding a quote character is not skipped -----
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      # - uses: "x/y@v4"\n' > .github/workflows/ci.yml
commit add-workflow-commented-quoted-decoy
check "# comment holding a quote is examined, not skipped" pins fail

# --- 46. pins: an unpinned uses: under a nested workflows/sub/*.yaml -------
setup
mkdir -p .github/workflows/sub
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@v4\n' > .github/workflows/sub/ci.yaml
commit add-workflow-nested-yaml-unpinned
check "unpinned uses: under .github/workflows/sub/ci.yaml" pins fail

# --- 47. pins: a job-level reusable workflow call pinned to a SHA: allowed --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    uses: o/r/.github/workflows/x.yml@11d5960a326750d5838078e36cf38b85af677262\n' > .github/workflows/ci.yml
commit add-workflow-reusable-pinned
check "job-level reusable workflow call, pinned to a SHA" pins pass

# --- 48. pins: an unpinned uses: is still found when run from ci/ ----------
# Regression: `pins` now reads from the repository root (git rev-parse
# --show-toplevel), so invoking it with cwd=ci/ must not vacuously pass by
# looking for .github/workflows/ under ci/.github/workflows/ instead.
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@v4\n' > .github/workflows/ci.yml
commit add-workflow-tag-for-subdir-cwd
out="$(cd ci && ./lane-gate.sh pins baseline HEAD 2>&1)"; rc=$?
name="pins from ci/ subdirectory still checks .github/workflows/"
if [ $rc -ne 0 ]; then
  printf 'ok    %-46s %s=%s\n' "$name" pins fail; pass_count=$((pass_count+1))
else
  printf 'NOT OK %-45s %s: expected fail got pass\n' "$name" pins
  printf '%s\n' "$out" | sed 's/^/        | /'
  fail_count=$((fail_count+1))
fi

printf '\n%s passed, %s failed\n' "$pass_count" "$fail_count"
[ "$fail_count" -eq 0 ]
