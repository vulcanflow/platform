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
#
# The scanner assumes a key and its colon share a line. That holds in block
# context but not in a flow mapping, where a line break or a comment may come
# between them. A key is spelled the literal `uses`, with an escape, as an
# alias, or as an explicit `?` key. It has no other spelling, because an
# unescaped line break inside a quoted key adds a space or a newline to it.
# Each spelling is refused unless its colon is on its line: an explicit key in
# any position; the others with the colon on the line or later.
# What git grep cannot read as lines is refused outright: a line break it does
# not split on, a file it reports as binary, a file whose blob holds a NUL byte
# (decided from the blob, never from git's attributes), a symlink or submodule,
# and a path holding a colon.
# ---------------------------------------------------------------------------

# `uses` as a whole word, then an optional closing quote and a colon, anywhere
# on the line (block key, flow collection, after an anchor or tag).
USES_SCOPE_RE='(^|[^[:alnum:]_])uses["'\'']?[[:space:]]*:'
# One `uses:` occurrence; group 2 is the rest of the line after it.
USES_KEY_RE='(^|[^[:alnum:]_])uses["'\'']?[[:space:]]*:[[:space:]]*(.*)$'
# The value. A quoted one ends at its closing quote (a doubled '' continues a
# single-quoted one). A plain one ends only at whitespace: , } ] and quotes are
# legal in a git ref name, so cutting there would check only a prefix of the ref.
DQ_VALUE_RE='^"([^"]*)"([^"]|$)'
SQ_VALUE_RE="^'([^']*)'([^']|\$)"
PLAIN_VALUE_RE='^[^[:space:]]*'
# The pin as written. The path holds no backslash, which a double-quoted value
# would decode into something else (`\x40` is an @).
PIN_RE='^[A-Za-z0-9][A-Za-z0-9-]*/[A-Za-z0-9_.-]+(/[^@[:space:]\\]+)?@[0-9a-f]{40}$'
# A line whose first non-blank character is # is a comment, unless it continues
# a quoted scalar from an earlier line; such a line holds the closing quote.
COMMENT_RE='^[[:space:]]*#'
# Keys that need no `uses` before a colon on the line: an explicit key, block
# (`? k`, `- ? k`) or flow (`[? k`, `{? k`, `, ? k`), whose text can be a block
# scalar or run on to later lines; an alias used as a key (`*k : v`); and a
# double-quoted key holding an escape (`"u\x73es": v`).
EXPLICIT_KEY_RE='(^[[:space:]]*(-[[:space:]]+)*|[[{,][[:space:]]*)\?([[:space:]]|$)'
ALIAS_KEY_RE='(^|[[:space:][{,])\*[^][{},:[:space:]]+[[:space:]]*:'
ESCAPED_KEY_RE='"[^"]*\\[^"]*"[[:space:]]*:'
# The same keys with the colon on a later line, which a flow mapping allows
# (`{ uses` then `: v }` on the next line). Nothing but blanks and a comment can
# follow such a key on its line. The literal word is matched wherever it ends a
# line, as USES_SCOPE_RE is. An escaped or alias key is matched only where a
# flow mapping can start a key: at the start of a line, after { [ or , or after
# a tag or anchor. That keeps out shell such as `printf "%s\n"` in a run: block.
FLOW_KEY_AT='(^[[:space:]]*|[[{,][[:space:]]*|[!&][^[:space:]]*[[:space:]]+)'
SPLIT_KEY_RE='(^|[^[:alnum:]_])uses["'\'']?[[:space:]]*(#.*)?$|'"$FLOW_KEY_AT"'("[^"]*\\[^"]*"|\*[^][{},:[:space:]]+)[[:space:]]*(#.*)?$'
# The first line of a double-quoted key continued by an escaped line break
# (`"us\` then `es": v`). Every line break in a key spelled `uses` must be
# escaped, the first one included, and the first sits on the line holding the
# opening quote.
ESCAPED_BREAK_RE="$FLOW_KEY_AT"'"[^"]*\\[[:space:]]*$'
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
  local top tree path oid nul n=0 bad=() line rest ref found
  # Read from the tree at HEAD_REF, as inline-tests does, so the verdict is the
  # commit's and not the working copy's, and from the repository root, so the
  # pathspec does not shrink to nothing when run from a subdirectory.
  top="$(git rev-parse --show-toplevel)"
  wf_grep "$top" -nE "$USES_SCOPE_RE"

  while IFS= read -r line; do
    [ -n "$line" ] || continue
    split_hit "$line"
    if [[ $TEXT =~ $COMMENT_RE && $TEXT != *[\"\']* ]]; then continue; fi
    found=0; rest="$TEXT"
    while [[ $rest =~ $USES_KEY_RE ]]; do
      rest="${BASH_REMATCH[2]}"
      found=1; n=$((n + 1))
      if [[ $rest =~ $DQ_VALUE_RE || $rest =~ $SQ_VALUE_RE ]]; then
        ref="${BASH_REMATCH[1]}"
      else
        [[ $rest =~ $PLAIN_VALUE_RE ]]; ref="${BASH_REMATCH[0]}"
      fi
      [[ $ref =~ $PIN_RE ]] || bad+=("$LOC: ${ref:-<no value on this line>}")
    done
    # A selected line with no `uses:` on it. git grep reports a file it holds to
    # be binary (a `-diff` attribute, say) as "Binary file <ref>:<path> matches",
    # with no line, so LOC is not a location then; the file is refused here, and
    # by the blob check below as well if it holds a NUL. Otherwise git grep and
    # bash disagree on USES_SCOPE_RE, and the line is refused either way.
    if [ "$found" -eq 0 ]; then
      n=$((n + 1)); bad+=("$LOC: <a uses: this check could not read>")
    fi
  done <<< "$WF_OUT"

  # A line holding a construct the scanner cannot read is refused whatever else
  # it says. A comment line is skipped only when no line break hides behind it.
  wf_grep "$top" -nE -e "$EXPLICIT_KEY_RE" -e "$ALIAS_KEY_RE" -e "$ESCAPED_KEY_RE" \
    -e "$SPLIT_KEY_RE" -e "$ESCAPED_BREAK_RE" -e "$BREAK_RE"
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    split_hit "$line"
    if [[ $TEXT =~ $BREAK_RE ]]; then
      bad+=("$LOC: <a line break other than LF or CRLF>")
    elif [[ $TEXT =~ $COMMENT_RE && $TEXT != *[\"\']* ]]; then
      continue
    elif [[ $TEXT =~ $EXPLICIT_KEY_RE ]]; then
      bad+=("$LOC: <an explicit ? key>")
    elif [[ $TEXT =~ $ALIAS_KEY_RE ]]; then
      bad+=("$LOC: <an alias used as a key>")
    elif [[ $TEXT =~ $ESCAPED_KEY_RE ]]; then
      bad+=("$LOC: <a double-quoted key holding an escape>")
    elif [[ $TEXT =~ $SPLIT_KEY_RE ]]; then
      bad+=("$LOC: <a line ending in uses, an alias or a quoted escape, which may be a key whose colon is on a later line>")
    elif [[ $TEXT =~ $ESCAPED_BREAK_RE ]]; then
      bad+=("$LOC: <a double-quoted key continued by an escaped line break>")
    else
      bad+=("$LOC: <a line this check could not read>")
    fi
  done <<< "$WF_OUT"

  # git grep skips a symlink and a submodule, so neither may stand on the way to
  # .github/workflows/ or under it, and split_hit cannot split a path holding a
  # colon. ls-tree prints <mode> <type> <object>\t<path>, the same paths git
  # grep searched; a colon is never quoted away.
  # A workflow is text when its blob holds no NUL byte. YAML may also be UTF-16
  # or UTF-32, where every ASCII character, `uses` included, carries a NUL, and
  # where the line passes above match nothing. The blob decides, not git grep -I
  # or git's binary heuristic: both follow the diff attribute, which a
  # .gitattributes in the tree under check can set.
  tree="$(git -C "$top" ls-tree "$HEAD_REF" -- .github)"
  tree+=$'\n'"$(git -C "$top" ls-tree -r -t "$HEAD_REF" -- .github/workflows)"
  while IFS= read -r line; do
    path="${line#*$'\t'}"
    case "$path" in
      *:*) bad+=("$path: <a path holding a colon>") ;;
    esac
    case "$line" in
      ''|'040000 '*) ;;
      '100644 '*|'100755 '*)
        case "$path" in
          .github/workflows/*)
            oid="${line%%$'\t'*}"; oid="${oid##* }"
            nul="$(git -C "$top" cat-file blob "$oid" | tr -dc '\000' | wc -c)" ||
              fail "pins: could not read $path at $HEAD_REF."
            [ "$nul" -eq 0 ] || bad+=("$path: <not a text file>") ;;
        esac ;;
      *) bad+=("$path: <a symlink or submodule>") ;;
    esac
  done <<< "$tree"

  if [ ${#bad[@]} -gt 0 ]; then
    printf '\n  uses: references not pinned to a commit SHA, or not readable by this check:\n' >&2
    printf '    %s\n' "${bad[@]}" >&2
    fail "Every uses: under .github/workflows/ must name a full 40-character commit SHA.

  A tag or branch is mutable, so re-pointing it upstream silently changes what CI
  executes. Write <owner>/<repo>[/<path>]@<40 lowercase hex>, optionally followed by
  a '# <tag>' comment, and resolve the SHA the way README.md '## Pins' describes.
  A local ./ action or a docker:// image has no commit SHA and is refused as well;
  admitting one is a change to this gate, on its own pull request.

  Every uses: on a line is checked, including one inside a string or a comment,
  and so is a line that ends in the word uses. Keep the key, its colon and its
  value on one line. An unquoted value ends only at whitespace, so in a flow
  mapping write { uses: <ref> } or quote the value.

  The check reads lines, not YAML, so it refuses what it cannot read: an
  explicit ? key, an alias used as a key, a double-quoted key holding an
  escape or continued by an escaped line break, a key whose colon is on a
  later line, a line break other than LF or CRLF, a file that is not text, a
  symlink or submodule, and a path holding a colon. No workflow needs these;
  write the key as plain uses: with its colon, and name the file without a
  colon. A line that only looks like such a key, a step name or shell line
  ending in uses, an alias or a quoted escape, is refused too; reword it."
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
