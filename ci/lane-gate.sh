#!/usr/bin/env bash
# lane-gate.sh — mechanical enforcement of the VulcanFlow delivery lanes (ADR-0005).
#
# Every VulcanFlow agent reaches GitHub through one shared identity, so the lanes
# cannot be enforced by CODEOWNERS or by counting approvals. They are enforced here,
# by partitioning the diff on paths instead of on people.
#
#   partition      a PR may not contain both production source and test files
#   erosion        tests may not be ignored, deleted, or thinned out
#   inline-tests   no #[cfg(test)] modules inside production source
#   pins           every workflow `uses:` names a full commit SHA
#   all            run all four
#
# Usage: ci/lane-gate.sh <subcommand> [base-ref] [head-ref]
# Defaults to origin/${GITHUB_BASE_REF:-main}...HEAD.

set -euo pipefail

BASE_REF="${2:-origin/${GITHUB_BASE_REF:-main}}"
HEAD_REF="${3:-HEAD}"

fail() { printf '\n\033[31mFAIL\033[0m %s\n' "$1" >&2; exit 1; }
pass() { printf '\033[32mPASS\033[0m %s\n' "$1"; }
note() { printf '      %s\n' "$1"; }

# ---------------------------------------------------------------------------
# Path classification
#
# Order matters: a path is tested against TEST patterns before PROD patterns, so
# crates/vf-core/tests/foo.rs is TEST and not PROD.
# ---------------------------------------------------------------------------

classify_path() {
  case "$1" in
    # --- GATE: the enforcement mechanism itself ---
    ci/lane-gate.sh|ci/lane-gate-test.sh|.github/workflows/lane-gate.yml) echo GATE ;;
    .tekton/tasks/lane-gate.yaml|.tekton/tasks/lane-gate-selftest.yaml|.tekton/pipelines/lane-gate.yaml|.tekton/kustomization.yaml|.tekton/kustomization.yml|.tekton/Kustomization) echo GATE ;;

    # --- TEST: owned by the test authors (Scribe, Ledger) ---
    tests/*|crates/*/tests/*|fuzz/*|conformance/*) echo TEST ;;
    *"/testdata/"*|*"/golden/"*|*.golden|*.snap) echo TEST ;;

    # --- PROD: owned by the coding agents (Forge, Anvil, Kiln) ---
    crates/*/src/*|crates/*/build.rs|crates/*/Cargo.toml) echo PROD ;;
    Cargo.toml|Cargo.lock|rust-toolchain.toml) echo PROD ;;

    # --- NEUTRAL: may accompany any lane ---
    *) echo NEUTRAL ;;
  esac
}

# One path per line. A deleted path is listed, and a rename is listed under both
# its old and its new path, so deleting a GATE or TEST file, or renaming one away
# to a NEUTRAL name, is classified the same as changing it. -M turns rename
# detection on whatever diff.renames is set to locally. -M never reports a copy (a
# copied file is an ordinary A row), so R is the only record that carries two paths.
# C stays in --diff-filter with no C* branch below on purpose: a copy record, if
# one ever appears, must be seen and rejected (fail closed) rather than dropped
# from the listing (fail open).
#
# -z makes every path reach classify_path as itself. Without it git C-quotes a
# path that holds a non-ASCII byte, a '"', a '\' or a control character, so
# crates/vf-core/tests/café.rs arrives as "crates/vf-core/tests/caf\303\251.rs"
# and its leading '"' makes it NEUTRAL. -c core.quotePath=false is not enough: it
# stops quoting only the non-ASCII bytes, and a '"', '\', tab or newline is still
# quoted. With -z git prints each path verbatim and ends every field with a NUL,
# which no path can contain.
#
# cmd_partition reads one path per line, so a newline inside a path is printed as
# '?'. Otherwise crates/a<newline>b/src/lib.rs would split into crates/a and
# b/src/lib.rs, both NEUTRAL, and hide a PROD file. No lane pattern contains a '?'
# or a newline, so the class does not change. A status other than A, D, M, R or T,
# or a record cut short, is a shape this parser does not know, so it fails closed
# instead of reading the rest of the stream out of step.
changed_files() {
  git diff --name-status -z -M --diff-filter=ACDMRT "$BASE_REF...$HEAD_REF" \
    | while IFS= read -r -d '' status; do
        case "$status" in
          R*)     n=2 ;;
          [ADMT]) n=1 ;;
          *)      fail "partition: git diff printed status '$status', which changed_files does not parse" ;;
        esac
        while [ "$n" -gt 0 ]; do
          IFS= read -r -d '' path || fail "partition: git diff record for status '$status' ends early"
          printf '%s\n' "${path//$'\n'/?}"
          n=$((n - 1))
        done
      done
}

# ---------------------------------------------------------------------------
# 1. partition — lanes 2, 3 and 5
# ---------------------------------------------------------------------------

cmd_partition() {
  local files prod=() test=() gate=() neutral=0
  files="$(changed_files)"
  if [ -z "$files" ]; then pass "partition: empty diff"; return 0; fi

  while IFS= read -r f; do
    [ -n "$f" ] || continue
    case "$(classify_path "$f")" in
      PROD) prod+=("$f") ;;
      TEST) test+=("$f") ;;
      GATE) gate+=("$f") ;;
      NEUTRAL) neutral=$((neutral + 1)) ;;
    esac
  done <<< "$files"

  note "production files: ${#prod[@]}   test files: ${#test[@]}   gate files: ${#gate[@]}   neutral: $neutral"

  if [ ${#prod[@]} -gt 0 ] && [ ${#test[@]} -gt 0 ]; then
    printf '\n  production source in this diff:\n' >&2
    printf '    %s\n' "${prod[@]}" >&2
    printf '\n  test files in this diff:\n' >&2
    printf '    %s\n' "${test[@]}" >&2
    fail "This pull request crosses a lane: it changes both production source and tests.

  Writing tests (lane 2) and writing production code (lane 3) are held by different
  agents, and a red test is a code defect that routes back to lane 3 — it is never
  edited into green (lane 5). Those rules are unenforceable if one pull request can
  carry both halves, so this check refuses the combination outright.

  Split it into two pull requests:
    - the test change, authored by Scribe (unit/property/fuzz) or Ledger
      (integration/conformance/e2e), carrying no production source;
    - the production change, authored by Forge, Anvil or Kiln, carrying no tests.

  If the test asserts behaviour the TDD never promised, that is a spec change, not a
  test fix: Atlas amends the TDD or files an ADR first, and only then does the
  original test author rewrite the test on a test-only pull request. See ADR-0005."
  fi

  if [ ${#gate[@]} -gt 0 ] && { [ ${#prod[@]} -gt 0 ] || [ ${#test[@]} -gt 0 ]; }; then
    printf '\n  gate files in this diff:\n' >&2
    printf '    %s\n' "${gate[@]}" >&2
    if [ ${#prod[@]} -gt 0 ]; then
      printf '\n  production source in this diff:\n' >&2
      printf '    %s\n' "${prod[@]}" >&2
    fi
    if [ ${#test[@]} -gt 0 ]; then
      printf '\n  test files in this diff:\n' >&2
      printf '    %s\n' "${test[@]}" >&2
    fi
    fail "This pull request changes the lane gate itself and also changes source or tests.

  A change to ci/lane-gate.sh, .github/workflows/lane-gate.yml or the Tekton lane gate
  under .tekton/ goes on its own pull request, so that weakening the gate is never
  bundled with the change it would let through. See ADR-0005."
  fi

  pass "partition: no lane crossing"
}

# ---------------------------------------------------------------------------
# 2. erosion — tests may not be ignored, deleted or thinned out
# ---------------------------------------------------------------------------

TEST_MARKER_RE='#\[test\]|#\[tokio::test|#\[test_case|#\[rstest|proptest!|fuzz_target!'

count_markers() {
  # Counts test declarations across the whole tree at a revision. Counting the
  # whole tree, rather than the diff, is what catches a deleted test file and a
  # test quietly moved out of the suite.
  #
  # `git grep` exits non-zero when it matches nothing, which is the ordinary case
  # for a revision with no Rust in it at all. Under `set -e -o pipefail` that
  # would abort the run with no verdict, so the miss is absorbed here.
  local n
  n="$( { git grep -hoE "$TEST_MARKER_RE" "$1" -- '*.rs' 2>/dev/null || true; } \
        | wc -l | tr -d ' ' )"
  printf '%s' "${n:-0}"
}

count_diff_lines() {
  # $1 = '+' for added lines or '-' for removed; $2 = pattern.
  local n
  n="$( { git diff "$BASE_REF...$HEAD_REF" -- '*.rs' 2>/dev/null || true; } \
        | { grep -E "^\\$1[^$1]" || true; } \
        | { grep -cE "$2" || true; } )"
  printf '%s' "${n:-0}"
}

cmd_erosion() {
  local base_rev base_count head_count added_ignores removed_tests

  added_ignores="$(count_diff_lines '+' '#\[ignore')"
  if [ "${added_ignores:-0}" -gt 0 ]; then
    git diff "$BASE_REF...$HEAD_REF" -- '*.rs' | grep -E '^\+[^+]' | grep -E '#\[ignore' >&2 || true
    fail "This pull request adds $added_ignores #[ignore] attribute(s).

  An ignored test is a deleted test that still looks present in the suite. If a test
  cannot pass, that is a code defect (lane 5) and it routes back to lane 3. See ADR-0005."
  fi

  removed_tests="$(count_diff_lines '-' "$TEST_MARKER_RE")"

  # The base is counted at the merge-base, the same base the `...` diffs use. The
  # base tip can be ahead of the head's own base: a reopened PR reuses a stale
  # merge ref, and main can move before the job fetches it. This needs the full
  # history of both refs (the workflow checks out with fetch-depth: 0).
  base_rev="$(git merge-base "$BASE_REF" "$HEAD_REF")" \
    || fail "erosion: no merge-base between $BASE_REF and $HEAD_REF, so there is no base to count.

  Either a ref does not resolve, the histories are unrelated, or the checkout is shallow.
  This check needs the full history of both refs."

  base_count="$(count_markers "$base_rev")"
  head_count="$(count_markers "$HEAD_REF")"
  note "test declarations: $base_count at base $(git rev-parse --short "$base_rev") (merge-base with $BASE_REF), $head_count at head (removed lines carrying a marker: ${removed_tests:-0})"

  if [ "$head_count" -lt "$base_count" ]; then
    fail "This pull request reduces the number of test declarations from $base_count to $head_count.

  Tests are only ever added. A test that is wrong is a spec question for Atlas, and it
  is rewritten by its original author on a test-only pull request after the TDD or an
  ADR has moved — never dropped to get a suite green. See ADR-0005."
  fi

  pass "erosion: test count did not fall ($base_count -> $head_count)"
}

# ---------------------------------------------------------------------------
# 3. inline-tests — keeps the path partition sound
# ---------------------------------------------------------------------------

cmd_inline_tests() {
  local hits
  # In Rust a #[cfg(test)] module lives in the same file as the code it tests, so
  # that one file is both PROD and TEST and the partition above cannot separate
  # them. Unit tests therefore live in crates/<crate>/tests/ and are written
  # against the public API.
  hits="$(git grep -lE '#\[cfg\(test\)\]' "$HEAD_REF" -- 'crates/*/src/*.rs' 2>/dev/null || true)"
  if [ -n "$hits" ]; then
    printf '\n  files with an inline test module:\n' >&2
    printf '    %s\n' "$hits" >&2
    fail "Production source contains #[cfg(test)] module(s).

  An inline test module puts a test and the code it tests in one file, which makes that
  file both production source and test at once — the path partition cannot separate
  them, and a coding agent editing its own tests becomes invisible to the gate.

  Put unit tests in crates/<crate>/tests/ and write them against the crate's public
  API. If a crate genuinely needs to assert on a private item, that is an ADR-0005
  revisit trigger, not a local exception. See ADR-0005."
  fi
  pass "inline-tests: no #[cfg(test)] modules in production source"
}

# ---------------------------------------------------------------------------
# 4. pins — every workflow `uses:` names a commit, never a tag or branch
#
# This is a line scanner, not a YAML parser, so it accepts one spelling of a
# pin and refuses any line that could spell another (VFL-485 Q3). GitHub
# matches the key by the exact string `uses`, and YAML builds a key's string
# from bytes on one line, save for a double-quoted escape or escaped line break.
#   1. A line whose bytes hold `uses` is a comment line or exactly
#      `[- ]uses: <pin>[ # <comment>]`, and holds no quote: a line that closes
#      a quoted scalar opened on an earlier line can carry a real key after the
#      quote, behind what reads as a pin or a comment. The value holds no
#      , [ ] { or }, which in a flow collection would end it early, leaving
#      GitHub a prefix of what was checked.
#   2. No line ends in a backslash, which inside double quotes joins the next
#      line with nothing between.
#   3. No \x, \u or \U, the only escapes that can spell a letter.
#   4. What git grep cannot read as lines is refused: a line break it does not
#      split on, a file it reports as binary, a file whose blob holds a NUL
#      byte (decided from the blob, never from git's attributes), a symlink or
#      submodule, and a path holding a colon.
# ---------------------------------------------------------------------------

# Rule 1; group 2 is the value. The pin's path holds no backslash, which a
# double-quoted value would decode into something else (`\x40` is an @).
CANON_RE='^[[:space:]]*(- )?uses: ([^][{},[:space:]]+)([[:space:]]+#.*)?$'
PIN_RE='^[A-Za-z0-9][A-Za-z0-9-]*/[A-Za-z0-9_.-]+(/[^@[:space:]\\]+)?@[0-9a-f]{40}$'
COMMENT_RE='^[[:space:]]*#'
# Rule 2 (before LF or CRLF) and rule 3.
END_ESCAPE_RE=$'\\\\\r?$'
LETTER_ESCAPE_RE='\\[xuU]'
# A line break YAML may honour and git grep does not: a lone CR, NEL, LS or PS.
# A `uses:` behind one would sit on what git grep reads as a comment line.
BREAK_RE=$'\r.|\xc2\x85|\xe2\x80\xa8|\xe2\x80\xa9'

# wf_grep <top> <git grep args...> — git grep .github/workflows/ in the tree at
# HEAD_REF into WF_OUT. Exit 1 is no match, which includes a tree without
# workflows; anything higher is a failure to read the tree.
wf_grep() {
  local top="$1" rc=0; shift
  WF_OUT="$(git -C "$top" grep "$@" "$HEAD_REF" -- '.github/workflows/')" || rc=$?
  if [ "$rc" -gt 1 ]; then
    fail "pins: could not read .github/workflows/ at $HEAD_REF (git grep exit $rc)."
  fi
}

# split_hit <line> — split one `git grep -n` line, <ref>:<path>:<n>:<text>, into
# LOC (<path>:<n>) and TEXT. It splits on the first two colons after <ref>, so
# a colon in the path would move the rest of the path into TEXT, where a `#`
# could pass for a comment. cmd_pins refuses such a path.
split_hit() {
  local l="${1#"$HEAD_REF:"}"
  LOC="${l%%:*}"; l="${l#*:}"
  LOC="$LOC:${l%%:*}"; TEXT="${l#*:}"
}

cmd_pins() {
  # Bytes, not characters, for git grep and for bash's own matching alike.
  local -x LC_ALL=C
  local top listed path shown oid nul n=0 bad=() line ref
  # Read from the tree at HEAD_REF, as inline-tests does, so the verdict is the
  # commit's and not the working copy's, and from the repository root, so the
  # pathspec does not shrink to nothing when run from a subdirectory.
  top="$(git rev-parse --show-toplevel)"
  wf_grep "$top" -nE -e uses -e "$END_ESCAPE_RE" -e "$LETTER_ESCAPE_RE" -e "$BREAK_RE"

  while IFS= read -r line; do
    [ -n "$line" ] || continue
    split_hit "$line"
    TEXT="${TEXT%$'\r'}"
    if [[ $TEXT =~ $BREAK_RE ]]; then
      bad+=("$LOC: <a line break other than LF or CRLF>")
    elif [[ $TEXT =~ $END_ESCAPE_RE ]]; then
      bad+=("$LOC: <a line ending in a backslash>")
    elif [[ $TEXT =~ $LETTER_ESCAPE_RE ]]; then
      bad+=("$LOC: <a \\x, \\u or \\U escape>")
    elif [[ $TEXT != *uses* ]]; then
      # git grep and bash disagree, or git grep printed "Binary file <ref>:<path>
      # matches", with no line, for a file git's attributes call binary.
      bad+=("$LOC: <a line this check could not read>")
    elif [[ $TEXT == *[\"\']* ]]; then
      bad+=("$LOC: <uses on a line holding a quote>")
    elif [[ $TEXT =~ $COMMENT_RE ]]; then
      continue
    elif [[ $TEXT =~ $CANON_RE ]]; then
      n=$((n + 1)); ref="${BASH_REMATCH[2]}"
      [[ $ref =~ $PIN_RE ]] || bad+=("$LOC: $ref")
    else
      bad+=("$LOC: <uses outside the form [- ]uses: <pin>>")
    fi
  done <<< "$WF_OUT"

  # git grep skips a symlink and a submodule, so neither may stand on the way to
  # .github/workflows/ or under it, and split_hit cannot split a path holding a
  # colon. ls-tree -z prints <mode> <type> <object>\t<path>\0, the same paths
  # git grep searched, each as stored. Without -z git C-quotes a path holding a
  # byte above 0x7f, a double quote, a backslash or a control character, and a
  # quoted path misses the .github/workflows/ pattern below, so its blob would
  # go unread; core.quotePath=false still quotes all but the first. The last
  # record, end, holds no tab, so no ls-tree record can pass for it; without it,
  # a listing failed. A path is reported %q-quoted, never raw.
  # A workflow is text when its blob holds no NUL byte. YAML may also be UTF-16
  # or UTF-32, where every ASCII character, `uses` included, carries a NUL, and
  # where the line passes above match nothing. The blob decides, not git grep -I
  # or git's binary heuristic: both follow the diff attribute, which a
  # .gitattributes in the tree under check can set.
  listed=0
  while IFS= read -r -d '' line; do
    if [ "$line" = end ]; then listed=1; continue; fi
    path="${line#*$'\t'}"
    printf -v shown '%q' "$path"
    case "$path" in
      *:*) bad+=("$shown: <a path holding a colon>") ;;
    esac
    case "$line" in
      '040000 '*) ;;
      '100644 '*|'100755 '*)
        case "$path" in
          .github/workflows/*)
            oid="${line%%$'\t'*}"; oid="${oid##* }"
            nul="$(git -C "$top" cat-file blob "$oid" | tr -dc '\000' | wc -c)" ||
              fail "pins: could not read $shown at $HEAD_REF."
            [ "$nul" -eq 0 ] || bad+=("$shown: <not a text file>") ;;
        esac ;;
      *) bad+=("$shown: <a symlink or submodule>") ;;
    esac
  done < <(git -C "$top" ls-tree -z "$HEAD_REF" -- .github &&
           git -C "$top" ls-tree -z -r -t "$HEAD_REF" -- .github/workflows &&
           printf 'end\0')
  [ "$listed" -eq 1 ] || fail "pins: could not list .github/ at $HEAD_REF."

  if [ ${#bad[@]} -gt 0 ]; then
    printf '\n  uses: references not pinned to a commit SHA, or not readable by this check:\n' >&2
    printf '    %s\n' "${bad[@]}" >&2
    fail "Every uses: under .github/workflows/ must name a full 40-character commit SHA.

  A tag or branch is mutable, so re-pointing it upstream silently changes what CI
  executes. The check reads lines, not YAML, and accepts one form, on one line:
      [- ]uses: <owner>/<repo>[/<path>]@<40 lowercase hex>[ # <tag>]
  with no quote on the line and no , [ ] { or } in the value. Resolve the SHA as
  README.md '## Pins' describes. Any other line holding the bytes uses is
  refused unless it is a comment without a quote; reword a step name or shell
  line that says uses. Refused anywhere: a line ending in a backslash, a \\x, \\u
  or \\U escape, a line break other than LF or CRLF, a file that is not text, a
  symlink or submodule, and a path holding a colon. A local ./ action or a
  docker:// image has no commit SHA and is refused too; admitting one is a
  change to this gate, on its own pull request."
  fi

  if [ "$n" -eq 0 ]; then
    pass "pins: no uses: references under .github/workflows/"
  else
    pass "pins: $n uses: reference(s), all pinned to a commit SHA"
  fi
}

# ---------------------------------------------------------------------------

case "${1:-all}" in
  partition)    cmd_partition ;;
  erosion)      cmd_erosion ;;
  inline-tests) cmd_inline_tests ;;
  pins)         cmd_pins ;;
  all)          cmd_partition; cmd_erosion; cmd_inline_tests; cmd_pins ;;
  *) printf 'usage: %s {partition|erosion|inline-tests|pins|all} [base-ref] [head-ref]\n' "$0" >&2; exit 2 ;;
esac
