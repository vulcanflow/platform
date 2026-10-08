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

#   The cases below (49-56) are regression fixtures for VFL-122 review round 2
#   (1b91879): the line scanner only ever caught a `uses:` whose scope line
#   held the literal word `uses` before a colon; each form below loads as
#   `uses: evil/act@v4` in js-yaml 4.3.1 but never matched that scan, so it
#   passed `pins` on 53f326e.

# --- 49. pins: a lone CR after a comment hides a step: refused -------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      # c\r      - uses: evil/act@v4\n' > .github/workflows/ci.yml
commit add-workflow-lone-cr-after-comment
check "lone CR after a comment hides an unpinned uses:" pins fail

# --- 50. pins: an explicit key spelled as a block scalar: refused ----------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - ? >-\n          uses\n        : evil/act@v4\n' > .github/workflows/ci.yml
commit add-workflow-explicit-key-block-scalar
check "explicit ? key spelled as a block scalar" pins fail

# --- 51. pins: a double-quoted key holding a backslash escape: refused -----
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - "u\\x73es": evil/act@v4\n' > .github/workflows/ci.yml
commit add-workflow-escaped-key
check "double-quoted key holding a backslash escape" pins fail

# --- 52. pins: an alias used as a key: refused ------------------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - x: &k uses\n      - *k : evil/act@v4\n' > .github/workflows/ci.yml
commit add-workflow-alias-key
check "alias (*k) used as a key" pins fail

# --- 53. pins: a UTF-16 workflow file: refused ------------------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: evil/act@v4\n' | iconv -f utf-8 -t utf-16 > .github/workflows/ci.yml
commit add-workflow-utf16
check "UTF-16 workflow file (git sees it as binary)" pins fail

# --- 54. pins: the workflow file is a symlink to content outside .github/: refused --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: evil/act@v4\n' > outside-workflow.yml
ln -s ../../outside-workflow.yml .github/workflows/ci.yml
commit add-workflow-symlink-to-outside-file
check "workflow file is a symlink to a file outside .github/" pins fail

# --- 55. pins: a CRLF workflow, pinned uses: plus a commented-out uses:: allowed --
setup
mkdir -p .github/workflows
printf 'on: push\r\njobs:\r\n  build:\r\n    steps:\r\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\r\n      # - uses: x/y@v4\r\n' > .github/workflows/ci.yml
commit add-workflow-crlf-pinned-and-commented
check "CRLF workflow: pinned uses: plus a commented-out uses:" pins pass

# --- 56. pins: a run: block holding *.rs, a ?) case arm and \n: allowed ----
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n      - run: |\n          ls *.rs\n          case $x in ?) ;; esac\n          printf "%%s\\n" "$x"\n' > .github/workflows/ci.yml
commit add-workflow-run-block-shell-metachars
check "run: block holding *.rs, a ?) case arm and a backslash escape" pins pass

#   The cases below (57-67) are discretionary extras for the same VFL-122
#   review round (1b91879): flow-form and symlink/submodule variants the
#   architect called out as optional, plus a few boundary checks confirming
#   the new refusals don't overreach onto lines that were always fine.

# --- 57. pins: a NEL line break after a comment hides a step: refused ------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      # c\xc2\x85      - uses: evil/act@v4\n' > .github/workflows/ci.yml
commit add-workflow-nel-after-comment
check "NEL line break after a comment hides an unpinned uses:" pins fail

# --- 58. pins: a LS line break after a comment hides a step: refused -------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      # c\xe2\x80\xa8      - uses: evil/act@v4\n' > .github/workflows/ci.yml
commit add-workflow-ls-after-comment
check "LS line break after a comment hides an unpinned uses:" pins fail

# --- 59. pins: a flow explicit key, { ? uses : v }: refused ----------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - { ? uses : evil/act@v4 }\n' > .github/workflows/ci.yml
commit add-workflow-flow-explicit-key
check "flow explicit { ? uses : ... } key" pins fail

# --- 60. pins: a double-quoted "uses" key with no escape, unpinned: refused --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - "uses": evil/act@v4\n' > .github/workflows/ci.yml
commit add-workflow-quoted-uses-key-unpinned
check "quoted \"uses\" key with no escape, unpinned value" pins fail

# --- 61. pins: a flow alias used as a key, {*k : v}: refused ---------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - x: &k uses\n      - {*k : evil/act@v4}\n' > .github/workflows/ci.yml
commit add-workflow-flow-alias-key
check "flow {*k : ...} alias key" pins fail

# --- 62. pins: .github itself is a symlink: refused -------------------------
setup
mkdir -p real-github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' > real-github/workflows/ci.yml
ln -s real-github .github
commit add-github-dir-symlink
check ".github itself is a symlink" pins fail

# --- 63. pins: .github/workflows itself is a symlink: refused ---------------
setup
mkdir -p .github real-workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' > real-workflows/ci.yml
ln -s ../real-workflows .github/workflows
commit add-workflows-dir-symlink
check ".github/workflows itself is a symlink" pins fail

# --- 64. pins: a gitlink (submodule) under .github/workflows/: refused -----
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' > .github/workflows/ci.yml
git add -A >/dev/null
git update-index --add --cacheinfo 160000,11d5960a326750d5838078e36cf38b85af677262,.github/workflows/sub
git commit -qm add-workflows-gitlink
check "gitlink (submodule) under .github/workflows/" pins fail

# --- 65. pins: a quote-less comment holding ?, * and : beside a pinned step: allowed --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n      # what? yes, ? maybe *k : no\n' > .github/workflows/ci.yml
commit add-workflow-quoteless-comment-with-triggers
check "quote-less comment holding ?, * and : beside a pinned step" pins pass

# --- 66. pins: an empty workflow file beside a pinned one: allowed ---------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' > .github/workflows/ci.yml
: > .github/workflows/empty.yml
commit add-empty-workflow-file
check "empty workflow file beside a pinned one" pins pass

# --- 67. pins: an executable (100755) workflow file: allowed ---------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' > .github/workflows/ci.yml
chmod +x .github/workflows/ci.yml
commit add-executable-workflow
check "executable (100755) workflow file" pins pass

#   The cases below (68-72) are regression fixtures for VFL-122 review round 2
#   (c977082, M2): split_hit splits each `git grep -n` line on the first two
#   colons after the ref, so a colon in the path moved the rest of the path
#   into the line text, and when that tail began with `#` the line passed for
#   a comment and was skipped. .github/workflows/a:b:#.yml holding
#   `uses: evil/act@v4` gave `PASS pins: no uses: references`, exit 0, on
#   c977082. 68f37be refuses any path under .github/workflows/ that holds a
#   colon, and case 71 closes the PS (U+2029) fixture gap the same review (L8)
#   found alongside it.

# --- 68. pins: a workflow path holding a colon, unpinned uses: (M2 reproduction): refused --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: evil/act@v4\n' > ".github/workflows/a:b:#.yml"
commit add-workflow-colon-path-unpinned
check "workflow path holding a colon, unpinned uses: (M2 reproduction)" pins fail

# --- 69. pins: same colon path, a correctly pinned uses:: refused by the path, by design --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' > ".github/workflows/a:b:#.yml"
commit add-workflow-colon-path-pinned
check "workflow path holding a colon, correctly pinned uses: (refused by the path)" pins fail

# --- 70. pins: a directory holding a colon under .github/workflows/: refused --
setup
mkdir -p ".github/workflows/d:x:#"
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: evil/act@v4\n' > ".github/workflows/d:x:#/w.yml"
commit add-workflow-colon-dir
check "directory holding a colon under .github/workflows/" pins fail

# --- 71. pins: a PS (U+2029) line break after a comment hides a step: refused --
# No existing fixture exercised PS; BREAK_RE already matches it (cases 49, 57
# and 58 cover CR, NEL and LS), so this closes a coverage gap and is not a
# claim that 68f37be changed this behaviour.
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      # c\xe2\x80\xa9      - uses: evil/act@v4\n' > .github/workflows/ci.yml
commit add-workflow-ps-after-comment
check "PS (U+2029) line break after a comment hides an unpinned uses:" pins fail

# --- 72. pins: a quoted path holding a double quote but no colon: allowed --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n      # comment\n' > '.github/workflows/q"x.yml'
commit add-workflow-quoted-path-no-colon
check "quoted path holding a double quote but no colon" pins pass

#   The cases below (73-77) are discretionary extras for the same review round
#   (VFL-472): path-colon boundary variants the task called out as optional.

# --- 73. pins: a:b: #.yml (space before the hash) path, unpinned uses:: refused --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: evil/act@v4\n' > ".github/workflows/a:b: #.yml"
commit add-workflow-colon-space-hash-path
check "workflow path a:b: #.yml (space before the hash), unpinned uses:" pins fail

# --- 74. pins: a colon-holding path is still refused when run from ci/ ----
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: evil/act@v4\n' > ".github/workflows/a:b:#.yml"
commit add-workflow-colon-path-for-subdir-cwd
out="$(cd ci && ./lane-gate.sh pins baseline HEAD 2>&1)"; rc=$?
name="pins from ci/ subdirectory still refuses a colon-holding path"
if [ $rc -ne 0 ]; then
  printf 'ok    %-46s %s=%s\n' "$name" pins fail; pass_count=$((pass_count+1))
else
  printf 'NOT OK %-45s %s: expected fail got pass\n' "$name" pins
  printf '%s\n' "$out" | sed 's/^/        | /'
  fail_count=$((fail_count+1))
fi

# --- 75. pins: a colon-holding path with no uses: key at all: refused -----
setup
mkdir -p .github/workflows
printf 'just plain text, no uses key here\n' > ".github/workflows/notes:x"
commit add-workflow-colon-path-no-uses
check "colon-holding workflow path with no uses: key at all" pins fail

# --- 76. pins: a non-ASCII path (quoted by git), no colon: allowed --------
setup
mkdir -p .github/workflows
F=$'.github/workflows/\xc3\xa9.yml'
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n      # comment\n' > "$F"
commit add-workflow-nonascii-path
check "non-ASCII workflow path (quoted by git), no colon" pins pass

# --- 77. pins: a pinned uses: value separated by a literal TAB: allowed ---
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses:\tactions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' > .github/workflows/ci.yml
commit add-workflow-tab-after-uses-colon
check "pinned uses: value separated by a literal TAB" pins pass

#   The cases below (78-86) are regression fixtures for VFL-122 review round 3
#   (c74e16e, M3): the scanner assumes a key and its colon share a line, which
#   holds in block context but not in a flow mapping, where a line break or a
#   comment may come between an implicit key and its colon.
#   `steps: [ { name: checkout, uses` then `: evil/act@v4, with: { ref: main } } ]`
#   on the next line gave PASS, exit 0, on ced2971, while GitHub's own
#   @actions/workflow-parser read it as uses: evil/act@v4 with no errors.
#   c74e16e refuses a literal, escaped or alias key whose colon sits on a
#   later line, a double-quoted key continued by an escaped line break, and a
#   backslash in a pinned path (which a double-quoted value would decode).

# --- 78. pins: a flow step uses key, colon on a later line (M3 reproduction): refused --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps: [ { name: checkout, uses\n      : evil/act@v4, with: { ref: main } } ]\n' > .github/workflows/ci.yml
commit add-workflow-flow-split-key-with-trailer
check "flow step uses key with its colon on a later line (M3 reproduction)" pins fail

# --- 79. pins: a quoted "uses" key, colon on a later line: refused ---------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps: [ { "uses"\n      : evil/act@v4 } ]\n' > .github/workflows/ci.yml
commit add-workflow-quoted-split-key
check "quoted \"uses\" key with its colon on a later line" pins fail

# --- 80. pins: a job-level uses key, colon on a later line: refused --------
setup
mkdir -p .github/workflows
printf 'on: push\njobs: { b: { uses\n  : evil/wf/.github/workflows/w.yml@main } }\n' > .github/workflows/ci.yml
commit add-workflow-job-level-split-key
check "job-level uses key with its colon on a later line" pins fail

# --- 81. pins: a double-quoted key continued by an escaped line break: refused --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps: [ { "us\\\n      es": evil/act@v4 } ]\n' > .github/workflows/ci.yml
commit add-workflow-escaped-key-line-break
check "double-quoted key continued by an escaped line break" pins fail

# --- 82. pins: an escaped "uses" key, colon on a later line: refused ------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps: [ { "u\\x73es"\n      : evil/act@v4 } ]\n' > .github/workflows/ci.yml
commit add-workflow-escaped-key-colon-split
check "escaped \"uses\" key with its colon on a later line" pins fail

# --- 83. pins: a backslash in a pinned path: refused -----------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: "evil/act/p\\x40v4@11d5960a326750d5838078e36cf38b85af677262"\n' > .github/workflows/ci.yml
commit add-workflow-backslash-in-pinned-path
check "backslash in a pinned path (a double-quoted value would decode it)" pins fail

# --- 84. pins: a pinned flow step, key colon and value on one line: allowed --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps: [ { uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262 } ]\n' > .github/workflows/ci.yml
commit add-workflow-flow-step-one-line-pinned
check "pinned flow step, key, colon and value all on one line" pins pass

# --- 85. pins: a flow step split across lines, key+colon+value together: allowed --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps: [ { name: a,\n      uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262 } ]\n' > .github/workflows/ci.yml
commit add-workflow-flow-step-split-colon-with-value
check "flow step split across lines, with key, colon and value together" pins pass

# --- 86. pins: a pinned step beside a run: printf holding an escaped quote: allowed --
# Confirms the escaped-key and split-key rules do not reach into a run: block:
# the closed, non-flow-key-position quote in printf "%s\n" a is not mistaken
# for a key.
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n      - run: printf "%%s\\n" a\n' > .github/workflows/ci.yml
commit add-workflow-run-printf-escaped-quote
check "pinned step beside a run: printf holding an escaped quote" pins pass

#   The cases below (87-91) are discretionary extras for the same review round
#   (c74e16e, M3): boundary variants the architect called out as optional.

# --- 87. pins: a split uses key followed by a comment on its own line: refused --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps: [ { uses # pick one\n      : evil/act@v4 } ]\n' > .github/workflows/ci.yml
commit add-workflow-split-key-comment-after
check "split uses key followed by a comment before the colon" pins fail

# --- 88. pins: a split uses key with a comment line between key and colon: refused --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps: [ { uses\n      # pick one\n      : evil/act@v4 } ]\n' > .github/workflows/ci.yml
commit add-workflow-split-key-comment-between
check "split uses key with a comment line between the key and the colon" pins fail

# --- 89. pins: a split uses key after an anchor: refused -------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps: [ { &a uses\n      : evil/act@v4 } ]\n' > .github/workflows/ci.yml
commit add-workflow-split-key-after-anchor
check "split uses key after an anchor" pins fail

# --- 90. pins: a job-level split key, CRLF line ends: refused --------------
setup
mkdir -p .github/workflows
printf 'on: push\r\njobs: { b: { uses\r\n  : evil/wf/.github/workflows/w.yml@main } }\r\n' > .github/workflows/ci.yml
commit add-workflow-job-level-split-key-crlf
check "job-level split key, CRLF line ends" pins fail

# --- 91. pins: a pinned step beside a run: echo ending in the word uses: refused --
# Accepted over-refusal (the same trade as case 86/L9): prose or shell that
# ends a line in the word uses is refused too, even though it is not a key.
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n      - run: echo "this step uses"\n' > .github/workflows/ci.yml
commit add-workflow-run-echo-this-step-uses
check "pinned step beside a run: echo line ending in the word uses" pins fail

printf '\n%s passed, %s failed\n' "$pass_count" "$fail_count"
[ "$fail_count" -eq 0 ]
