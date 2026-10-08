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

changed_files() {
  git diff --name-only --diff-filter=ACMRT "$BASE_REF...$HEAD_REF"
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
    fail "This pull request changes the lane gate itself and also changes source or tests.

  A change to ci/lane-gate.sh or .github/workflows/lane-gate.yml goes on its own pull
  request, so that weakening the gate is never bundled with the change it would let
  through. See ADR-0005."
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
  local base_count head_count added_ignores removed_tests

  added_ignores="$(count_diff_lines '+' '#\[ignore')"
  if [ "${added_ignores:-0}" -gt 0 ]; then
    git diff "$BASE_REF...$HEAD_REF" -- '*.rs' | grep -E '^\+[^+]' | grep -E '#\[ignore' >&2 || true
    fail "This pull request adds $added_ignores #[ignore] attribute(s).

  An ignored test is a deleted test that still looks present in the suite. If a test
  cannot pass, that is a code defect (lane 5) and it routes back to lane 3. See ADR-0005."
  fi

  removed_tests="$(count_diff_lines '-' "$TEST_MARKER_RE")"

  base_count="$(count_markers "$BASE_REF")"
  head_count="$(count_markers "$HEAD_REF")"
  note "test declarations: $base_count at base, $head_count at head (removed lines carrying a marker: ${removed_tests:-0})"

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
# This is a line scanner, not a YAML parser. USES_SCOPE_RE picks the lines to
# examine, and every `uses:` on an examined line must carry a pinned value.
# Both are wider than YAML's key positions on purpose: a `uses:` inside a
# string or a trailing comment is checked too, which can refuse a line but
# never admit one.
# ---------------------------------------------------------------------------

# `uses` as a whole word, then an optional closing quote and a colon, anywhere
# on the line (block key, flow collection, after an anchor or tag); or an
# explicit `? uses` key, whose value is on a later line and so is refused.
USES_SCOPE_RE='(^|[^[:alnum:]_])uses["'\'']?[[:space:]]*:|\?[[:space:]]+["'\'']?uses["'\'']?[[:space:]]*$'
# One `uses:` occurrence; group 2 is the rest of the line after it.
USES_KEY_RE='(^|[^[:alnum:]_])uses["'\'']?[[:space:]]*:[[:space:]]*(.*)$'
# The value. A quoted one ends at its closing quote (a doubled '' continues a
# single-quoted one). A plain one ends only at whitespace: , } ] and quotes are
# legal in a git ref name, so cutting there would check only a prefix of the ref.
DQ_VALUE_RE='^"([^"]*)"([^"]|$)'
SQ_VALUE_RE="^'([^']*)'([^']|\$)"
PLAIN_VALUE_RE='^[^[:space:]]*'
PIN_RE='^[A-Za-z0-9][A-Za-z0-9-]*/[A-Za-z0-9_.-]+(/[^@[:space:]]+)?@[0-9a-f]{40}$'
# A line whose first non-blank character is # is a comment, unless it continues
# a quoted scalar from an earlier line; such a line holds the closing quote.
COMMENT_RE='^[[:space:]]*#'

cmd_pins() {
  local top refs rc=0 n=0 bad=() line loc text rest ref found
  # Read from the tree at HEAD_REF, as inline-tests does, so the verdict is the
  # commit's and not the working copy's, and from the repository root, so the
  # pathspec does not shrink to nothing when run from a subdirectory. `git grep`
  # exits 1 on no match, which is a tree without workflows; anything higher is
  # a failure to read it.
  top="$(git rev-parse --show-toplevel)"
  refs="$(git -C "$top" grep -nE "$USES_SCOPE_RE" "$HEAD_REF" -- '.github/workflows/')" || rc=$?
  if [ "$rc" -gt 1 ]; then
    fail "pins: could not read .github/workflows/ at $HEAD_REF (git grep exit $rc)."
  fi

  while IFS= read -r line; do
    [ -n "$line" ] || continue
    line="${line#"$HEAD_REF:"}"
    loc="${line%%:*}"; line="${line#*:}"
    loc="$loc:${line%%:*}"; text="${line#*:}"
    if [[ $text =~ $COMMENT_RE && $text != *[\"\']* ]]; then continue; fi
    found=0; rest="$text"
    while [[ $rest =~ $USES_KEY_RE ]]; do
      rest="${BASH_REMATCH[2]}"
      found=1; n=$((n + 1))
      if [[ $rest =~ $DQ_VALUE_RE || $rest =~ $SQ_VALUE_RE ]]; then
        ref="${BASH_REMATCH[1]}"
      else
        [[ $rest =~ $PLAIN_VALUE_RE ]]; ref="${BASH_REMATCH[0]}"
      fi
      [[ $ref =~ $PIN_RE ]] || bad+=("$loc: ${ref:-<no value on this line>}")
    done
    if [ "$found" -eq 0 ]; then
      n=$((n + 1)); bad+=("$loc: <explicit ? uses key; value not on this line>")
    fi
  done <<< "$refs"

  if [ ${#bad[@]} -gt 0 ]; then
    printf '\n  uses: references not pinned to a commit SHA:\n' >&2
    printf '    %s\n' "${bad[@]}" >&2
    fail "Every uses: under .github/workflows/ must name a full 40-character commit SHA.

  A tag or branch is mutable, so re-pointing it upstream silently changes what CI
  executes. Write <owner>/<repo>[/<path>]@<40 lowercase hex>, optionally followed by
  a '# <tag>' comment, and resolve the SHA the way README.md '## Pins' describes.
  A local ./ action or a docker:// image has no commit SHA and is refused as well;
  admitting one is a change to this gate, on its own pull request.

  Every uses: on a line is checked, including one inside a string or a comment.
  Keep the key and its value on one line. An unquoted value ends only at
  whitespace, so in a flow mapping write { uses: <ref> } or quote the value."
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
