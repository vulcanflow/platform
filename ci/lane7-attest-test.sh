#!/usr/bin/env bash
# Local harness for ci/lane7-attest.sh — the lane-7 attestation detector.
#
# Lane 2 (VUL-56). Written from the spec before the detector exists: the
# authority is plans/lane7-attest-detector-spec.md revision 3 (docs#35) and
# ADR-0005 §6.3 condition 4 / §6.5. Section references below are to that
# document unless they name ADR-0005.
#
# One `check` call per row of §9's 46-row fixture table, in table order. Each
# row builds a throwaway repository under mktemp -d, writes the non-git facts
# as flat files under LANE7_FIXTURE_DIR (§6.1), and asserts the complete set of
# ledger lines together with the exit code (§9.0).
#
# Every row is the §9.0 baseline with the one variable its comment names:
# merge shape, a well-formed six-key block, no trailers, no gate workflow in the
# tree, nothing under crates/**, both §6.2 credentials set, and every lookup the
# detector performs succeeding against the value the block names.
#
# usage: ci/lane7-attest-test.sh <path-to-ci/lane7-attest.sh>
set -uo pipefail

[ $# -eq 1 ] || { printf 'usage: %s <path-to-ci/lane7-attest.sh>\n' "$0" >&2; exit 2; }

# The Paperclip GitHub launcher blanks the git identity vars and shadows `git`
# on PATH; both have to go for a local fixture repo to work. Same preamble as
# ci/lane-gate-test.sh, for the same reason.
unset GIT_AUTHOR_NAME GIT_AUTHOR_EMAIL GIT_COMMITTER_NAME GIT_COMMITTER_EMAIL
unset GIT_CONFIG_GLOBAL GIT_CONFIG_SYSTEM GIT_CONFIG_COUNT
export PATH="/usr/bin:/bin:/usr/local/bin"
export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@example.com
export GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@example.com

# Resolved before the harness cd's into its fixture, so a relative argument works.
DET="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
# Exit 2 rather than reporting 46 identical "No such file or directory" rows. The
# harness makes the same distinction the detector does (§5): "I could not run"
# and "the thing I ran is wrong" are different events with different owners, and
# before the detector exists only the first of them is true.
[ -f "$DET" ] || { printf 'no detector at %s -- nothing to assert against\n' "$DET" >&2; exit 2; }
# pwd -P so the MODE line's <dir> compares equal to what we pass in (§3).
W="$(cd "$(mktemp -d)" && pwd -P)"
FX="$W/fx"
trap 'rm -rf "$W"' EXIT

pass_count=0; fail_count=0

# Well-formed 40-hex shas that are not any commit in a fixture repository. The
# seam is what makes them usable: under LANE7_FIXTURE_DIR the pull-request and
# check-run facts are keyed by sha from flat files, never from the object store
# (§4.3, answering N3), so a block may name a sha no fixture repo holds.
SHA_WRONG=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
SHA_OTHER=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb

# --- fixture construction ---------------------------------------------------

# new_repo — a fresh single-commit repository with the detector and the floor
# manifest in ci/, an empty fact tree, and the §9.0 baseline environment.
# Resets every variable a row can vary, so no row leaks into the next.
new_repo() {
  rm -rf "$W/r" "$FX"
  mkdir -p "$W/r" "$FX/pulls" "$FX/pull-head" "$FX/check-runs" "$FX/verdicts"
  cd "$W/r" || exit 1
  git init -q -b main .
  mkdir -p ci
  cp "$DET" ci/lane7-attest.sh; chmod +x ci/lane7-attest.sh
  manifest
  printf '# fixture\n' > README.md
  git add -A >/dev/null; git commit -qm 'chore: initialize fixture repository'
  export LANE7_FIXTURE_DIR="$FX"
  export LANE7_GITHUB_TOKEN=fixture-github-token
  export LANE7_PAPERCLIP_TOKEN=fixture-paperclip-token
}

# manifest [<infra-floor>] — the §2 floor manifest, beside the script because
# that is where §2 says the detector resolves it from. The four shas transcribe
# ADR-0005 §6.5. vulcanflow/nope is deliberately absent (row 27).
manifest() {
  cat > ci/lane7-attest-floors.txt <<EOF
# ci/lane7-attest-floors.txt — the ADR-0005 §6.5 floor manifest (fixture copy).
# Moving a floor is an amendment to ADR-0005 §6.5, not an edit to this file.
#
# <owner>/<repo>            <40-hex floor sha — the newest commit NOT asserted on>
vulcanflow/docs             b31ddfeca0ad88cb481929f7eacd64d7e8194ac0
vulcanflow/platform         41506ad3bea5282473207a725b00625c5f65e0aa
vulcanflow/infra            ${1:-304b300e01e0c9ad8210977d80986afe65517daa}
vulcanflow/vf-api           45d7ded9d32d0fff8e60a17c97c725c475748497
EOF
}

# shape_merge [<path>] — a true merge of a feature branch into main. Sets FEAT,
# the second parent, which is the head a §9.0 baseline block attests.
shape_merge() {
  local f="${1:-feature.txt}"
  git checkout -q -b feature
  mkdir -p "$(dirname "$f")"; printf 'feature\n' > "$f"
  git add -A >/dev/null; git commit -qm 'feat: the branch commit'
  FEAT="$(git rev-parse HEAD)"
  git checkout -q main
  git merge -q --no-ff --no-edit -m placeholder feature
}

# shape_squash [<path>] — a squash of a feature branch: one parent, so the head
# is reachable only through pulls/ then pull-head/ (§6.5 class 2). Sets FEAT.
shape_squash() {
  local f="${1:-feature.txt}"
  git checkout -q -b feature
  mkdir -p "$(dirname "$f")"; printf 'feature\n' > "$f"
  git add -A >/dev/null; git commit -qm 'feat: the branch commit'
  FEAT="$(git rev-parse HEAD)"
  git checkout -q main
  git merge -q --squash feature >/dev/null
  git commit -qm placeholder
}

# shape_direct — a commit that reached main by neither a merge nor a squash of a
# pull request, but which still has a branch sha a block could name. Built as a
# squash so the commit has one parent; `pulls/<sha>` = none is what makes it
# class 3 (rows 9, 42, 43). Sets FEAT.
shape_direct() { shape_squash "$@"; }

# finish <msgfile> — give the commit under test its message and publish SHA.
# --amend keeps both parents of a merge. --cleanup=verbatim keeps the message
# byte-exact, which rows 25 and 29 need because §7 is a rule about what follows
# the block and git's default cleanup rewrites exactly that.
finish() {
  git commit -q --amend --cleanup=verbatim -F "$1"
  SHA="$(git rev-parse HEAD)"
}

# facts <attested-head> — the §9.0 baseline fact files: every lookup the
# detector performs succeeds, keyed on the value the block names. Rows that vary
# one lookup call this and then overwrite or remove the single file they vary.
facts() {
  local h="$1"
  printf '1\n' > "$FX/pulls/$SHA"
  printf '%s\n' "$h" > "$FX/pull-head/1"
  printf 'lane-partition success\ntest-erosion success\ninline-tests success\ngate-self-test success\n' \
    > "$FX/check-runs/$h"
  printf 'Assay APPROVE %s\n'  "$h" > "$FX/verdicts/VUL-57"
  printf 'Warren APPROVE %s\n' "$h" > "$FX/verdicts/VUL-58"
}

# --- assertion --------------------------------------------------------------

# report <name> <expected-exit> <actual-exit> <expected> <actual> <raw-stdout>
report() {
  local name="$1" xrc="$2" rc="$3" want="$4" got="$5" raw="$6"
  if [ "$got" = "$want" ] && [ "$rc" = "$xrc" ]; then
    printf 'ok    %-56s exit=%s\n' "$name" "$rc"; pass_count=$((pass_count+1))
    return
  fi
  printf 'NOT OK %-55s expected exit %s got %s\n' "$name" "$xrc" "$rc"
  printf '       expected ledger:\n'; printf '%s\n' "$want" | sed 's/^/         | /'
  printf '       actual ledger:\n';   printf '%s\n' "$got"  | sed 's/^/         | /'
  printf '       stdout as emitted:\n'; printf '%s\n' "$raw" | sed 's/^/         | /'
  printf '       stderr:\n'; sed 's/^/         | /' "$W/err"
  fail_count=$((fail_count+1))
}

# ledger <raw-stdout> <line-prefix-pattern> — the comparable part of the ledger:
# the MODE line, which §3 requires to be the first stdout line unconditionally,
# then the matching lines with the prose after ' -- ' removed, sorted.
#
# Sorted, so §3's "the harness compares the set of stdout lines, not their
# order" holds. NOT de-duplicated, because §3 makes one-triple-one-line the
# detector's obligation rather than a harness tolerance, and row 46 asserts
# "exactly one" — `sort -u` would hide the duplicate it exists to catch.
ledger() {
  printf '%s\n' "$1" | sed -n 1p
  printf '%s\n' "$1" | grep -E "$2" | sed 's/ -- .*$//' | sort
}

# expected <line>... — the same shape for the expected side.
expected() {
  printf 'MODE fixture %s\n' "$FX"
  printf '%s\n' "$@" | grep -v '^$' | sort
}

# check <name> <sha> <expected-exit> [<expected-line>...]
#
# Compares only the `OK ` and `L7-` lines. It does not assert the absence of a
# RANGE line on `commit`: §3 ties RANGE to a repository being classified and §1
# gives `commit` one commit and no range, but neither forbids one, and a harness
# that reddens on a line the spec never ruled on is asserting its author's
# reading. Rows 27 and 28 assert RANGE where §9 does state it.
check() {
  local name="$1" sha="$2" xrc="$3"; shift 3
  local out rc
  out="$(./ci/lane7-attest.sh commit vulcanflow/docs "$sha" 2>"$W/err")"; rc=$?
  report "$name" "$xrc" "$rc" "$(expected "$@")" "$(ledger "$out" '^(OK |L7-)')" "$out"
}

# check_repo <name> <owner/repo> <expected-exit> [<expected-line>...]
# As check, but for the `repo` subcommand, and RANGE lines are compared too.
check_repo() {
  local name="$1" repo="$2" xrc="$3"; shift 3
  local out rc
  out="$(./ci/lane7-attest.sh repo "$repo" 2>"$W/err")"; rc=$?
  report "$name" "$xrc" "$rc" "$(expected "$@")" "$(ledger "$out" '^(RANGE |OK |L7-)')" "$out"
}

# --- 1. the baseline --------------------------------------------------------
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
check "1  the baseline" "$SHA" 0 "OK vulcanflow/docs $SHA"

# --- 2. squash, both n/a lines carrying their reason, no gate workflow -------
new_repo; shape_squash
cat > "$W/msg" <<EOF
chore: a documentation change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: n/a (no workflow on docs)
Lane-7-Ledger: n/a (no §25 identifier in scope)
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
check "2  squash, both n/a with a reason" "$SHA" 0 "OK vulcanflow/docs $SHA"

# --- 3. no Lane-7-* line at all ---------------------------------------------
# Class 1 suppresses 2 and 4-8. Class 3 survives (ADR-0005 §6.5) but is silent
# here, because pulls/ resolves: the commit did arrive by a pull request.
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change with no attestation

A one-line body.
EOF
finish "$W/msg"; facts "$FEAT"
check "3  no Lane-7-* line at all" "$SHA" 1 "L7-MISSING vulcanflow/docs $SHA"

# --- 4. Lane-7-Merged-By absent ---------------------------------------------
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
EOF
finish "$W/msg"; facts "$FEAT"
check "4  Lane-7-Merged-By absent" "$SHA" 1 "L7-KEY vulcanflow/docs $SHA"

# --- 5. Lane-7-Ledger absent ------------------------------------------------
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
check "5  Lane-7-Ledger absent" "$SHA" 1 "L7-KEY vulcanflow/docs $SHA"

# --- 6. true merge, Lane-7-Head != the second parent ------------------------
# Rule C (§4.3): the head is present and well-formed, so every check that reads
# it runs against the sha the block names. facts is keyed on that sha, so class
# 8 and the verdict lookup both succeed and class 2's finding stands alone.
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $SHA_WRONG
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $SHA_WRONG
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $SHA_WRONG
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$SHA_WRONG"
check "6  merge, head != second parent" "$SHA" 1 "L7-HEAD vulcanflow/docs $SHA"

# --- 7. squash, Lane-7-Head != pull-head/<n> --------------------------------
new_repo; shape_squash
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $SHA_WRONG
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $SHA_WRONG
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $SHA_WRONG
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$SHA_WRONG"
printf '%s\n' "$FEAT" > "$FX/pull-head/1"   # the pull request's real head
check "7  squash, head != pull-head/<n>" "$SHA" 1 "L7-HEAD vulcanflow/docs $SHA"

# --- 8. squash, pull-head/<n> absent ----------------------------------------
# The pull request was identified, so this is not L7-PR-UNCHECKED; the ref did
# not resolve, so class 2 cannot compare and reports that instead of L7-HEAD.
new_repo; shape_squash
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
rm "$FX/pull-head/1"
check "8  squash, pull-head/<n> absent" "$SHA" 1 "L7-HEAD-UNRESOLVABLE vulcanflow/docs $SHA"

# --- 9. one parent, pulls/<sha> = none --------------------------------------
# Class 3 in isolation: the block is well-formed, so class 1 never fires. No
# pull request was identified, so class 2 has nothing to compare and is silent.
new_repo; shape_direct
cat > "$W/msg" <<EOF
chore: a change that reached main directly

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
printf 'none\n' > "$FX/pulls/$SHA"
check "9  one parent, pulls = none" "$SHA" 1 "L7-NOT-PR vulcanflow/docs $SHA"

# --- 10. Lane-7-Gate: n/a with no parenthesised reason ----------------------
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: n/a
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
check "10 Lane-7-Gate: bare n/a" "$SHA" 1 "L7-GATE-BARE vulcanflow/docs $SHA"

# --- 11. Lane-7-Gate n/a on a tree that has the gate workflow --------------
# §9.2: the detector keys on the tree, not on the repository the reason names,
# so the harness asserts nothing about "platform" appearing here.
new_repo
mkdir -p .github/workflows; printf 'name: lane-gate\n' > .github/workflows/lane-gate.yml
git add -A >/dev/null; git commit -qm 'ci: add the lane gate'
shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: n/a (no workflow on platform)
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
check "11 Gate n/a, tree has lane-gate.yml" "$SHA" 1 "L7-GATE-VACUOUS vulcanflow/docs $SHA"

# --- 12. Lane-7-Gate: FAIL -------------------------------------------------
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: FAIL
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
check "12 Lane-7-Gate: FAIL" "$SHA" 1 "L7-GATE-VALUE vulcanflow/docs $SHA"

# --- 13. Lane-7-Ledger: n/a with no reason ---------------------------------
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: n/a
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
check "13 Lane-7-Ledger: bare n/a" "$SHA" 1 "L7-LEDGER-BARE vulcanflow/docs $SHA"

# --- 14. Ledger n/a on a commit touching crates/** -------------------------
# The one advisory code (§5), so the finding is reported and the exit code is 0.
new_repo; shape_merge crates/vf-core/src/lib.rs
cat > "$W/msg" <<EOF
feat(vf-core): a change under crates

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: n/a (no §25 identifier in scope)
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
check "14 Ledger n/a, touches crates/**" "$SHA" 0 "L7-LEDGER-VACUOUS vulcanflow/docs $SHA"

# --- 15. one Lane-7-Verdict-* line only ------------------------------------
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
check "15 one verdict line only" "$SHA" 1 "L7-VERDICT-COUNT vulcanflow/docs $SHA"

# --- 16. three Lane-7-Verdict-* lines --------------------------------------
# The third line repeats verdict 1 exactly, so every field on every line stays
# well-formed and the count is the only defect: the reviewers are both in
# {Assay, Warren}, all three dispositions are APPROVE, all three cite an issue
# and all three cover the head. The row therefore forces exactly one suppression
# beyond §4.3's table -- L7-VERDICT-COUNT over L7-VERDICT-DUP -- which is the
# minimum any third line can be built to demand, since three lines over a
# two-reviewer set cannot avoid both DUP and WHO. Raised with lane 1.
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Verdict-3: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
check "16 three verdict lines" "$SHA" 1 "L7-VERDICT-COUNT vulcanflow/docs $SHA"

# --- 17. two verdicts, both naming Assay ----------------------------------
# L7-VERDICT-DUP suppresses all four lookup codes (§4.3), so VUL-58 recording
# Warren rather than Assay produces no second finding.
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Assay    VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
check "17 two verdicts, both Assay" "$SHA" 1 "L7-VERDICT-DUP vulcanflow/docs $SHA"

# --- 18. a verdict naming an agent outside {Assay, Warren} ----------------
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Atlas    VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
check "18 a verdict naming Atlas" "$SHA" 1 "L7-VERDICT-WHO vulcanflow/docs $SHA"

# --- 19. a verdict reading REQUEST CHANGES --------------------------------
# Two words, so a detector that splits the line on whitespace reads "CHANGES"
# as the reviewer. The row expects L7-VERDICT-DISP alone, so the disposition
# check has to fire first and leave the rest of the line unread. Raised with
# lane 1 alongside row 16.
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: REQUEST CHANGES  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
check "19 a verdict reading REQUEST CHANGES" "$SHA" 1 "L7-VERDICT-DISP vulcanflow/docs $SHA"

# --- 20. a verdict reading APPROVED --------------------------------------
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVED  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
check "20 a verdict reading APPROVED" "$SHA" 1 "L7-VERDICT-DISP vulcanflow/docs $SHA"

# --- 21. a verdict citing no VUL-<n> -------------------------------------
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
check "21 a verdict citing no VUL-<n>" "$SHA" 1 "L7-VERDICT-ISSUE vulcanflow/docs $SHA"

# --- 22. covers != Lane-7-Head, both well-formed -------------------------
# §6.4's shape, and rule C's boundary against row 37 (§4.3). covers is a
# well-formed sha that is simply wrong, so the lookup runs -- keyed on the head
# the block names -- and succeeds; L7-VERDICT-COVERS stands alone. Squash shape,
# because ff1e2be has to equal the pull request's head for class 2 to be silent,
# and the seam lets a sha no fixture repo holds be that head.
new_repo; shape_squash
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: ff1e2be
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers 5168c5c
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers ff1e2be
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts ff1e2be
check "22 covers != head, both well-formed" "$SHA" 1 "L7-VERDICT-COVERS vulcanflow/docs $SHA"

# --- 23. Lane-7-Merged-By: CEO ------------------------------------------
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: CEO
EOF
finish "$W/msg"; facts "$FEAT"
check "23 Lane-7-Merged-By: CEO" "$SHA" 1 "L7-MERGED-BY vulcanflow/docs $SHA"

# --- 24. LANE7_PAPERCLIP_TOKEN unset, verdicts/ still complete ----------
# The finding is produced by the credential being absent and not by the fact
# being absent (§6.2), and it exits 1: a check that passes when it could not run
# is §6.2 corollary 4's shape.
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
unset LANE7_PAPERCLIP_TOKEN
check "24 LANE7_PAPERCLIP_TOKEN unset" "$SHA" 1 "L7-VERDICT-UNCHECKED vulcanflow/docs $SHA"

# --- 25. a well-formed block followed by two paragraphs of prose --------
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible

One more thing worth saying about this change.

And a second paragraph, below the block.
EOF
finish "$W/msg"; facts "$FEAT"
check "25 prose after the block" "$SHA" 1 "L7-NOT-TRAILING vulcanflow/docs $SHA"

# --- 26. two findings from two different present keys ------------------
# The row that forbids stopping at the first finding. Merged-By is absent and a
# verdict's covers is well-formed and wrong, so rule A suppresses nothing and
# the ledger carries both lines and no OK.
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $SHA_OTHER
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
EOF
finish "$W/msg"; facts "$FEAT"
check "26 Merged-By absent and covers wrong" "$SHA" 1 \
  "L7-KEY vulcanflow/docs $SHA" \
  "L7-VERDICT-COVERS vulcanflow/docs $SHA"

# --- 27. a repository in the scan set with no manifest row -------------
# The third field is the literal '-', and no RANGE line is emitted.
new_repo
check_repo "27 repo with no manifest row" vulcanflow/nope 1 "L7-NO-FLOOR vulcanflow/nope -"

# --- 28. the empty range ----------------------------------------------
# RANGE is emitted for every repository classified, not only the non-empty ones,
# so the ledger states its own scope (§3). Not a finding, so exit 0.
new_repo
manifest "$(git rev-parse main)"
check_repo "28 the empty range" vulcanflow/infra 0 "RANGE vulcanflow/infra 0"

# --- 29. the permitted trailers after the block -----------------------
# §7's leniency, in the exact shape GitHub's squash UI produces: the separator
# then Co-authored-by lines. The only row in the table that appends trailers.
new_repo; shape_squash
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible

---------

Co-authored-by: Assay <assay@example.com>
Co-authored-by: Warren <warren@example.com>
EOF
finish "$W/msg"; facts "$FEAT"
check "29 separator and Co-authored-by trailers" "$SHA" 0 "OK vulcanflow/docs $SHA"

# --- 30. Lane-7-Head absent ------------------------------------------
# Rule A (§4.3): the absent head suppresses class 2, L7-VERDICT-COVERS, all four
# verdict lookup codes and both class-8 codes, so L7-KEY stands alone. The
# exact-set comparison is what asserts "and no L7-VERDICT-* line": §4.2's rule 2
# is that no lookup is ever performed with an empty sha, and "" matching
# vacuously is the live false pass this row exists to catch.
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
check "30 Lane-7-Head absent" "$SHA" 1 "L7-KEY vulcanflow/docs $SHA"

# --- 31. Lane-7-Gate absent ------------------------------------------
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
check "31 Lane-7-Gate absent" "$SHA" 1 "L7-KEY vulcanflow/docs $SHA"

# --- 32. zero verdict lines, the other five keys present -------------
# The boundary two authors would otherwise guess differently: verdict lines are
# counted, the four single-valued keys are keyed. Zero is L7-VERDICT-COUNT and
# not L7-KEY.
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
check "32 zero verdict lines" "$SHA" 1 "L7-VERDICT-COUNT vulcanflow/docs $SHA"

# --- 33. verdicts/VUL-<n> absent -- the issue does not exist ---------
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
rm "$FX/verdicts/VUL-57"
check "33 verdicts/VUL-<n> absent" "$SHA" 1 "L7-VERDICT-ISSUE-MISSING vulcanflow/docs $SHA"

# --- 34. verdicts/VUL-<n> present and empty -- it records none -------
# Against row 33: the whole of the difference between the lookup not happening
# and the lookup happening and the answer being nothing (§6.1).
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
: > "$FX/verdicts/VUL-57"
check "34 verdicts/VUL-<n> present and empty" "$SHA" 1 "L7-VERDICT-NOT-FOUND vulcanflow/docs $SHA"

# --- 35. the APPROVE exists, recorded by the wrong agent -------------
# §6.2 corollary 2: one reviewer's approval attributed to the other is how two
# verdicts become one. The reviewer is checked at both ends (§4.2 rule 3).
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
printf 'Kiln APPROVE %s\n' "$FEAT" > "$FX/verdicts/VUL-57"
check "35 the APPROVE is recorded by Kiln" "$SHA" 1 "L7-VERDICT-MISATTRIBUTED vulcanflow/docs $SHA"

# --- 36. the repair for B1 -- co-occurrence is not a match -----------
# The disposition word APPROVE and the covered sha both appear in the issue, and
# no single record carries both (§4.2 rule 1). A detector that passes rows 1-35
# and fails this one is the detector that shipped the defect.
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
printf 'Assay REQUEST_CHANGES %s\nWarren APPROVE %s\n' "$FEAT" "$SHA_OTHER" > "$FX/verdicts/VUL-57"
check "36 APPROVE and the sha co-occur, no record has both" "$SHA" 1 \
  "L7-VERDICT-NOT-FOUND vulcanflow/docs $SHA"

# --- 37. a malformed covers value -----------------------------------
# Rule A's boundary against row 22 (§4.3): deadbeefzz is not a sha, so nothing
# can be keyed on it and the lookup does not run. Same expected set as 22, a
# different mechanism, and an implementation that confuses them fails one.
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers deadbeefzz
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
check "37 a malformed covers value" "$SHA" 1 "L7-VERDICT-COVERS vulcanflow/docs $SHA"

# --- 38. an attested PASS over a failing required check -------------
# Class 8: the lookup ran and did not confirm the PASS. Not "untrue" -- nothing
# confirmed it, which is what the detector can honestly say (§9.1).
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
printf 'lane-partition failure\n' > "$FX/check-runs/$FEAT"
check "38 attested PASS over a failing check" "$SHA" 1 "L7-GATE-UNCONFIRMED vulcanflow/docs $SHA"

# --- 39. the check-run lookup could not run ------------------------
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
rm "$FX/check-runs/$FEAT"
check "39 check-runs/<head> absent" "$SHA" 1 "L7-GATE-UNCHECKED vulcanflow/docs $SHA"

# --- 40. could not resolve is not no pull request -----------------
# Rule B (§4.3). The exact-set comparison is what asserts the absence of
# L7-NOT-PR: manufacturing the most serious finding in the set out of a lookup
# failure is §6.2 corollary 4 pointed the other way, and only a negative
# assertion catches it.
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
rm "$FX/pulls/$SHA"
check "40 pulls/<sha> absent" "$SHA" 1 "L7-PR-UNCHECKED vulcanflow/docs $SHA"

# --- 41. the API is the route, and the subject carries no suffix --
# With row 42, the only two rows that can fail an implementation which resolved
# the pull request by parsing (#n) out of the subject: this one goes red with a
# false L7-NOT-PR on a legitimate merge.
new_repo; shape_squash
cat > "$W/msg" <<EOF
chore: a change with no number in its subject

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
printf '30\n' > "$FX/pulls/$SHA"; printf '%s\n' "$FEAT" > "$FX/pull-head/30"
check "41 squash, pulls = 30, no (#n) in the subject" "$SHA" 0 "OK vulcanflow/docs $SHA"

# --- 42. the subject lies -------------------------------------
# The other half of 41: a subject suffix on a commit the API says reached main
# without a pull request. An implementation that trusts the suffix goes green.
new_repo; shape_direct
cat > "$W/msg" <<EOF
chore: a change that never had a pull request (#99)

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
printf 'none\n' > "$FX/pulls/$SHA"
check "42 the subject reads (#99), pulls = none" "$SHA" 1 "L7-NOT-PR vulcanflow/docs $SHA"

# --- 43. class 3 survives class 1's subsumption ---------------
# ADR-0005 §6.5's subsumption rule is unassertable without this row: a commit
# can be both unattested and direct-pushed, and class 1 must not swallow class 3.
new_repo; shape_direct
cat > "$W/msg" <<EOF
chore: an unattested change that never had a pull request

A one-line body.
EOF
finish "$W/msg"; facts "$FEAT"
printf 'none\n' > "$FX/pulls/$SHA"
check "43 no block and no pull request" "$SHA" 1 \
  "L7-MISSING vulcanflow/docs $SHA" \
  "L7-NOT-PR vulcanflow/docs $SHA"

# --- 44. an attested PASS on a commit with no check runs at all ---
# Present and empty: the lookup ran and the commit has no check runs. A required
# workflow that never ran is the more likely real defect of the two class-8
# cases, and the one §6.5 names first. Gate is PASS, not n/a, so the gate
# workflow in the tree does not make this L7-GATE-VACUOUS.
new_repo
mkdir -p .github/workflows; printf 'name: lane-gate\n' > .github/workflows/lane-gate.yml
git add -A >/dev/null; git commit -qm 'ci: add the lane gate'
shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
: > "$FX/check-runs/$FEAT"
check "44 check-runs/<head> present and empty" "$SHA" 1 "L7-GATE-UNCONFIRMED vulcanflow/docs $SHA"

# --- 45. LANE7_GITHUB_TOKEN unset, merge shape -----------------
# Fixture 24's shape pointed at the other credential: the facts are all present
# and complete, so the two UNCHECKED codes are produced by the credential being
# absent. No L7-VERDICT-* line, because LANE7_PAPERCLIP_TOKEN is still set and
# the head is the second parent -- a git fact needing no credential, which is
# also why both negative clauses here are satisfied vacuously and why row 46
# exists.
new_repo; shape_merge
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
unset LANE7_GITHUB_TOKEN
check "45 LANE7_GITHUB_TOKEN unset, merge" "$SHA" 1 \
  "L7-PR-UNCHECKED vulcanflow/docs $SHA" \
  "L7-GATE-UNCHECKED vulcanflow/docs $SHA"

# --- 46. LANE7_GITHUB_TOKEN unset, squash shape ----------------
# The non-vacuous half of 45. On a squash the head resolves through pulls then
# pull-head, so the absent credential means the pull-request number never exists
# and rule A's suppression of L7-HEAD-UNRESOLVABLE is real where row 8 shows it
# is otherwise reachable. This is also the only row that forces §3's
# one-triple-one-line rule: classes 2 and 3 both reach L7-PR-UNCHECKED here and
# the ledger must carry one line, which the un-deduplicated comparison in
# ledger() is what catches.
new_repo; shape_squash
cat > "$W/msg" <<EOF
chore: a change

A one-line body.

Lane-7-Head: $FEAT
Lane-7-Gate: PASS
Lane-7-Ledger: PASS
Lane-7-Verdict-1: APPROVE  Assay    VUL-57  covers $FEAT
Lane-7-Verdict-2: APPROVE  Warren   VUL-58  covers $FEAT
Lane-7-Merged-By: Crucible
EOF
finish "$W/msg"; facts "$FEAT"
unset LANE7_GITHUB_TOKEN
check "46 LANE7_GITHUB_TOKEN unset, squash" "$SHA" 1 \
  "L7-PR-UNCHECKED vulcanflow/docs $SHA" \
  "L7-GATE-UNCHECKED vulcanflow/docs $SHA"

printf '\n%s passed, %s failed\n' "$pass_count" "$fail_count"
[ "$fail_count" -eq 0 ]
