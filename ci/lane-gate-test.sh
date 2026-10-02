#!/usr/bin/env bash
# Local harness for ci/lane-gate.sh. Builds a tiny fixture repo and asserts each
# subcommand's verdict on a set of representative diffs.
#
# Fixtures are numbered to match ADR-0005's enumerations and every expected
# verdict comes from a record, never from reading ci/lane-gate.sh:
#   1-15  the original set (§4.2's path classification)
#   16-27 §4.5 "Re-derived over six GATE paths" — 16-19 and 23-27 are step A and
#         are here; 20-22 are step C and land after Forge extends
#         classify_path(), because they assert a refusal that does not yet exist
#         and gate-self-test is a required check (R10).
# At step A: 24 fixture diffs, 32 `check` invocations, 26 pass / 6 fail.
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
  mkdir -p ci crates/vf-core/src crates/vf-core/tests fuzz .github/workflows
  cp "$GATE" ci/lane-gate.sh; chmod +x ci/lane-gate.sh
  # The other five GATE paths of ADR-0005 §4.2 (as re-derived over six paths in
  # §4.5 "Re-derived over six GATE paths") exist in the baseline so a fixture can
  # build a *modification* diff over any of the six, not just over the one path
  # the gate is copied to. They are stubs on purpose: `cmd_partition()`
  # classifies on the path and never reads the contents, so a stub is the entire
  # requirement, and copying the real harness in would make the fixture
  # recursive. Nothing in the harness executes them.
  printf '#!/usr/bin/env bash\n# stub: ci/lane-gate-test.sh\n' > ci/lane-gate-test.sh
  printf '#!/usr/bin/env bash\n# stub: ci/pre-pr-review-verdict.sh\n' > ci/pre-pr-review-verdict.sh
  printf '#!/usr/bin/env bash\n# stub: ci/pre-pr-review-verdict-test.sh\n' > ci/pre-pr-review-verdict-test.sh
  chmod +x ci/lane-gate-test.sh ci/pre-pr-review-verdict.sh ci/pre-pr-review-verdict-test.sh
  printf 'name: lane-gate\n# stub: .github/workflows/lane-gate.yml\n' > .github/workflows/lane-gate.yml
  printf 'name: pre-pr-review-verdict\n# stub: .github/workflows/pre-pr-review-verdict.yml\n' \
    > .github/workflows/pre-pr-review-verdict.yml
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

# =====================================================================
# Fixtures 16-27 — ADR-0005 §4.5 "Re-derived over six GATE paths" (R8's
# first firing). §4.2's GATE row grows from three paths to six; the second
# triple is ci/pre-pr-review-verdict.sh, ci/pre-pr-review-verdict-test.sh
# and .github/workflows/pre-pr-review-verdict.yml.
#
# This block is **step A** of §4.5's three-step table: the nine fixtures
# whose verdict is identical before and after Forge extends
# `classify_path()`. Today the three verdict paths are NEUTRAL, and a
# NEUTRAL-only diff passes for the same reason a GATE-only diff does; after
# step B they are GATE and the same rows still pass. Every expected value
# below is the one in §4.5's enumeration table, not a value derived from
# reading ci/lane-gate.sh.
# =====================================================================

# --- 16. the verdict classifier alone: allowed -----------------------------
# NEUTRAL-only today, GATE-only after step B. Either way: no PROD, no TEST.
setup
printf '\n# tweak\n' >> ci/pre-pr-review-verdict.sh
commit verdict-script-only
check "verdict script alone" partition pass

# --- 17. the verdict harness alone: allowed --------------------------------
# Also asserts the `-test.sh` suffix does not pull the path into TEST: §4.2's
# TEST row is tests/**, crates/*/tests/**, fuzz/**, conformance/**,
# **/testdata/**, **/golden/**, *.golden, *.snap — and not a ci/ filename.
setup
printf '\n# tweak\n' >> ci/pre-pr-review-verdict-test.sh
commit verdict-harness-only
check "verdict harness alone" partition pass

# --- 18. the verdict workflow alone: allowed -------------------------------
setup
printf '\n# tweak\n' >> .github/workflows/pre-pr-review-verdict.yml
commit verdict-workflow-only
check "verdict workflow alone" partition pass

# --- 19. the whole verdict triple in one diff: allowed ---------------------
# The second triple's own "gate pair in one diff" — a classifier, its harness
# and its workflow travelling together is permitted, which is the hole §4.5
# limb 2 exists to compensate for on the first triple and which the second
# triple does not yet have a mechanism for (§4.5 sub-check 2).
setup
printf '\n# tweak\n' >> ci/pre-pr-review-verdict.sh
printf '\n# tweak\n' >> ci/pre-pr-review-verdict-test.sh
printf '\n# tweak\n' >> .github/workflows/pre-pr-review-verdict.yml
commit verdict-triple
check "verdict triple together" partition pass

# --- 20-22. step C, deliberately absent ------------------------------------
# §4.5's table rows 20 ("verdict script + prod"), 21 ("verdict harness +
# test") and 22 ("verdict workflow + prod") assert a *refusal*. The
# classifier does not perform that refusal until step B lands, and
# `gate-self-test` is a required status check, so writing them here would be a
# red required check that cannot merge — R10. §4.5 supersedes §6.6's
# "fixtures written first" clause for exactly these three rows and puts them
# after the classifier change. They are step C, on a separate Scribe pull
# request, and they take the harness to 27 fixtures / 35 invocations.

# --- 23. all six GATE paths in one diff: allowed on all three sub-commands -
# §4.5's central observation, asserted: gate=6, prod=0, test=0.
# `cmd_partition()` never compares GATE members to each other and never
# compares the gate count to anything but zero, so any non-empty subset of
# GATE, alone, passes — for any size of GATE. All three sub-commands are
# checked because that is what "passes" has to mean for a diff carrying both
# classifiers and both harnesses.
setup
printf '\n# tweak\n' >> ci/lane-gate.sh
printf '\n# tweak\n' >> ci/lane-gate-test.sh
printf '\n# tweak\n' >> .github/workflows/lane-gate.yml
printf '\n# tweak\n' >> ci/pre-pr-review-verdict.sh
printf '\n# tweak\n' >> ci/pre-pr-review-verdict-test.sh
printf '\n# tweak\n' >> .github/workflows/pre-pr-review-verdict.yml
commit all-six-gate-paths
check "all six gate paths" partition pass
check "all six gate paths" erosion pass
check "all six gate paths" inline-tests pass

# --- 24. a cross-triple GATE diff: allowed -------------------------------
# A diff shape that did not exist when GATE held one triple, and nothing looks
# at it: erosion greps '*.rs' and sees no shell or YAML, inline-tests is
# unaffected, and §4.5 limb 2's monotonicity, floor and sentinel are specified
# over the lane-gate triple by name. §4.6 is the only control. Asserted here
# so it is on the record as permitted-by-design rather than found later.
setup
printf '\n# tweak\n' >> ci/lane-gate.sh
printf '\n# tweak\n' >> .github/workflows/pre-pr-review-verdict.yml
commit cross-triple-gate
check "cross-triple gate diff" partition pass

# --- 25. the lane-gate classifier and its harness in one diff: allowed ---
# §4.5 limb 2's subject, asserted rather than assumed. Shared with §4.5's own
# step 1; counted once (§4.5's fourth note on the enumeration table).
setup
printf '\n# tweak\n' >> ci/lane-gate.sh
printf '\n# tweak\n' >> ci/lane-gate-test.sh
commit gate-pair
check "gate pair in one diff" partition pass

# --- 26. the lane-gate workflow alone: allowed ---------------------------
# The third member of the first triple, which setup() could not reach before.
# Shared with §4.5's own step 1; counted once.
setup
printf '\n# tweak\n' >> .github/workflows/lane-gate.yml
commit gate-workflow-only
check "gate workflow alone" partition pass

# --- 27. near-miss: a NEUTRAL ci/ script may accompany production source --
# The three new GATE literals must be matched exactly, not as a prefix or a
# glob. ci/pre-pr-review-verdict-helper.sh stays NEUTRAL, so it may travel
# with PROD. Without this row, `ci/pre-pr-review-verdict*)` passes every other
# fixture here and silently pulls arbitrary future ci/ scripts into GATE.
setup
printf '#!/usr/bin/env bash\n# helper\n' > ci/pre-pr-review-verdict-helper.sh
printf 'pub fn add(a:i32,b:i32)->i32{a+b}\npub fn sub(a:i32,b:i32)->i32{a-b}\n' > crates/vf-core/src/lib.rs
commit near-miss-plus-prod
check "near-miss ci script + prod" partition pass

printf '\n%s passed, %s failed\n' "$pass_count" "$fail_count"
[ "$fail_count" -eq 0 ]
