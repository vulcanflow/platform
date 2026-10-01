#!/usr/bin/env bash
# lane7-attest.sh — the lane-7 attestation detector (ADR-0005 §6.3 condition 4).
#
# §3.1 means one GitHub identity serves all nine agents, so nothing in GitHub
# distinguishes a Crucible merge from any other and lane 7 has no mechanical
# backstop. §6.3 condition 4 answers that by requiring every merge to a default
# branch to carry an attestation block in its commit message — which does not
# prevent an unattested merge, but makes one visible from `main`'s own history
# afterwards. This script is the reader §6.3 says is "a follow-up item in
# `platform`; it is not yet written". It is that item.
#
# It reports exactly the three defects §6.3 names as visible to a `git log`
# reader, and nothing else is allowed to change the exit status:
#
#   d1  no attestation block on a merge at or after the first attested merge
#   d2  Lane-7-Head is not the commit actually merged
#   d3  a verdict line citing an issue where no such verdict exists, or citing a
#       `covers <sha>` that is not Lane-7-Head
#
# Anything else this script notices — a bare `n/a` on a repository that has a
# workflow, a Lane-7-Merged-By that is not Crucible, a verdict whose disposition
# is not APPROVE, the two verdicts naming the same reviewer — is printed as an
# `advisory` and does NOT affect the exit status. Promoting one of those to a
# defect is an ADR-0005 amendment, not a change to this script.
#
# What it cannot see, and does not pretend to:
#
#   - §6.3 condition 3's "zero unresolved blocking findings" clause is not
#     evidenced by the block and cannot be reconstructed from `main`. An `OK`
#     line means the block is well formed and internally consistent. It does not
#     mean the review it attests to was sound.
#   - d3's issue-existence half is the only check that reaches outside git. With
#     no Paperclip credential it reports UNCHECKED — never a pass. A check that
#     silently passes when it cannot run is the failure mode this whole record
#     exists to close (§6.2 corollary 4 is the same shape).
#   - For a squash merge, git carries no independent record of the head that was
#     merged, so d2 has nothing to compare the line against. If the repository's
#     `refs/pull/*/head` refs have been fetched the subject's `(#N)` resolves one
#     and d2 is checked; otherwise d2 reports UNCHECKED for that commit. Fetch
#     them with:
#       git fetch origin '+refs/pull/*/head:refs/pull/*/head'
#
# Pre-attestation history. ADR-0005 §7 item 10: the history before 2026-10-01
# carries no attestations and cannot be made to. The scan therefore starts at the
# first commit in first-parent order that carries a block, and says how many
# earlier commits it skipped. The one exception is the KNOWN_DEFECTIVE table
# below, which is a recorded fact about a named commit (§6.4) and not something
# this script infers.
#
# Usage:
#   ci/lane7-attest.sh scan [--repo <dir>] [--strict] [--ignore-recorded] [<ref>]
#   ci/lane7-attest.sh classify [--commit <sha>] [--merged-head <sha>]
#                               [--subject <text>] [--has-workflow yes|no]
#                               [--strict] < commit-message
#   ci/lane7-attest.sh help
#
# `scan` defaults to `origin/main` and to the current directory. `classify`
# reads one commit message on stdin and needs no git and no credential; it is the
# fixture surface for ci/lane7-attest-test.sh.
#
# Exit: 0 when every reported commit is OK; 1 when any d1/d2/d3 is found; 1 also
# on UNCHECKED when --strict is given; 2 on a usage error.

set -euo pipefail

# ---------------------------------------------------------------------------
# Recorded defects — facts from the design record, not inferences
#
# ADR-0005 §6.4: docs#24 was merged at 20:59:26 on 2026-10-01 over reviewer #1's
# unresolved REQUEST CHANGES and with no reviewer #2 verdict in existence. It
# predates condition 4, so it carries no block and the d1 rule above would skip
# it. It is listed here so the audit shows it rather than hides it behind
# "pre-attestation".
# ---------------------------------------------------------------------------

known_defective_note() {
  case "$1" in
    b40201b23d5514998f28e1c9e564da33dc4eb263)
      echo 'merged over an unresolved REQUEST CHANGES and with no reviewer #2 verdict (recorded, ADR-0005 §6.4)' ;;
    *) echo '' ;;
  esac
}

# ---------------------------------------------------------------------------
# Output
# ---------------------------------------------------------------------------

if [ -t 1 ]; then C_RED=$'\033[31m'; C_GRN=$'\033[32m'; C_YEL=$'\033[33m'; C_OFF=$'\033[0m'
else C_RED=''; C_GRN=''; C_YEL=''; C_OFF=''; fi

usage() {
  # The header comment is the documentation; print it up to the first blank line
  # that is not a comment.
  sed -n '2,/^$/p' "$0" | sed 's/^#\{1,\} \{0,1\}//'
  exit "${1:-2}"
}

# Per-commit accumulators, reset by classify_message.
FINDINGS=()      # "d1|<text>", "d2|<text>", "d3|<text>"
UNCHECKS=()      # "<text>"
ADVISORIES=()    # "<text>"
REVIEWERS=()     # the reviewer named by each parsed verdict line

add_defect()   { FINDINGS+=("$1|$2"); }
add_uncheck()  { UNCHECKS+=("$1"); }
add_advisory() { ADVISORIES+=("$1"); }

# ---------------------------------------------------------------------------
# Paperclip lookup — the only part that reaches outside git
#
# Returns one of: found | missing-issue | missing-verdict | unchecked:<reason>
# ---------------------------------------------------------------------------

API_READY=''   # '', 'yes', or 'no:<reason>'

api_ready() {
  if [ -n "$API_READY" ]; then [ "$API_READY" = yes ]; return; fi
  if [ -z "${PAPERCLIP_API_KEY:-}" ]; then
    API_READY='no:no PAPERCLIP_API_KEY in the environment'
  elif [ -z "${PAPERCLIP_API_URL:-}" ]; then
    API_READY='no:no PAPERCLIP_API_URL in the environment'
  elif [ -z "${PAPERCLIP_COMPANY_ID:-}" ]; then
    API_READY='no:no PAPERCLIP_COMPANY_ID in the environment'
  elif ! command -v curl >/dev/null 2>&1; then
    API_READY='no:curl is not installed'
  elif ! command -v python3 >/dev/null 2>&1; then
    API_READY='no:python3 is not installed'
  else
    API_READY=yes
  fi
  [ "$API_READY" = yes ]
}

api_reason() { printf '%s' "${API_READY#no:}"; }

# verdict_lookup <issue-identifier> <disposition> <covered-sha>
verdict_lookup() {
  local ident="$1" disp="$2" sha="$3" base
  base="${PAPERCLIP_API_URL%/}"; base="${base%/api}"
  PC_IDENT="$ident" PC_DISP="$disp" PC_SHA="$sha" PC_BASE="$base" python3 - <<'PY' 2>/dev/null || echo "unchecked:the Paperclip lookup failed"
import json, os, sys, urllib.parse, urllib.request

base = os.environ["PC_BASE"]
ident = os.environ["PC_IDENT"]
disp = os.environ["PC_DISP"].upper()
sha = os.environ["PC_SHA"].lower()
key = os.environ["PAPERCLIP_API_KEY"]
company = os.environ["PAPERCLIP_COMPANY_ID"]


def get(path):
    req = urllib.request.Request(base + path, headers={"Authorization": "Bearer " + key})
    with urllib.request.urlopen(req, timeout=20) as r:
        return json.load(r)


def rows(doc, *keys):
    if isinstance(doc, list):
        return doc
    for k in keys:
        v = doc.get(k)
        if isinstance(v, list):
            return v
    return []


try:
    q = urllib.parse.quote(ident)
    found = get(f"/api/companies/{company}/issues?q={q}")
except Exception as exc:  # noqa: BLE001 - reported, never swallowed into a pass
    print(f"unchecked:{type(exc).__name__} querying the Paperclip API")
    sys.exit(0)

issue = next(
    (i for i in rows(found, "issues", "data", "items") if str(i.get("identifier", "")).upper() == ident.upper()),
    None,
)
if issue is None:
    print("missing-issue")
    sys.exit(0)

# The verdict is whatever the reviewer recorded on their own lane-6 issue, so the
# evidence is the issue's own text plus its comments. Match on the disposition
# word and the covered sha appearing together in one body; a prefix is enough
# because the block may carry a short sha.
bodies = [str(issue.get("description") or "")]
try:
    bodies += [str(c.get("body") or "") for c in rows(get(f"/api/issues/{issue['id']}/comments"), "comments", "data", "items")]
except Exception as exc:  # noqa: BLE001
    print(f"unchecked:{type(exc).__name__} reading the comments of {ident}")
    sys.exit(0)

for body in bodies:
    low = body.lower()
    if disp.lower() in low and sha[:7] in low:
        print("found")
        sys.exit(0)

print("missing-verdict")
PY
}

# ---------------------------------------------------------------------------
# The block parser and the three checks
#
# classify_message <commit-sha> <merged-head|''> <head-known: yes|no|squash>
#                  <has-workflow: yes|no|unknown>
# Message on stdin. Populates FINDINGS / UNCHECKS / ADVISORIES.
# ---------------------------------------------------------------------------

classify_message() {
  local commit="$1" merged_head="$2" head_known="$3" has_workflow="$4"
  FINDINGS=(); UNCHECKS=(); ADVISORIES=(); REVIEWERS=()

  local msg line n_lines=0
  msg="$(cat)"

  local l7_head='' l7_gate='' l7_ledger='' l7_mergedby='' v1='' v2=''
  while IFS= read -r line; do
    case "$line" in
      Lane-7-Head:*)      l7_head="${line#Lane-7-Head:}";          n_lines=$((n_lines+1)) ;;
      Lane-7-Gate:*)      l7_gate="${line#Lane-7-Gate:}";          n_lines=$((n_lines+1)) ;;
      Lane-7-Ledger:*)    l7_ledger="${line#Lane-7-Ledger:}";      n_lines=$((n_lines+1)) ;;
      Lane-7-Verdict-1:*) v1="${line#Lane-7-Verdict-1:}";          n_lines=$((n_lines+1)) ;;
      Lane-7-Verdict-2:*) v2="${line#Lane-7-Verdict-2:}";          n_lines=$((n_lines+1)) ;;
      Lane-7-Merged-By:*) l7_mergedby="${line#Lane-7-Merged-By:}"; n_lines=$((n_lines+1)) ;;
      Lane-7-*) add_advisory "unrecognised attestation line: ${line%%:*}" ;;
    esac
  done <<< "$msg"

  l7_head="$(trim "$l7_head")"; l7_gate="$(trim "$l7_gate")"
  l7_ledger="$(trim "$l7_ledger")"; l7_mergedby="$(trim "$l7_mergedby")"
  v1="$(trim "$v1")"; v2="$(trim "$v2")"

  # --- d1: no attestation block ------------------------------------------
  if [ "$n_lines" -eq 0 ]; then
    add_defect d1 'no attestation block (ADR-0005 §6.3 condition 4)'
    return 0
  fi

  local missing=()
  [ -n "$l7_head" ]     || missing+=(Lane-7-Head)
  [ -n "$l7_gate" ]     || missing+=(Lane-7-Gate)
  [ -n "$l7_ledger" ]   || missing+=(Lane-7-Ledger)
  [ -n "$v1" ]          || missing+=(Lane-7-Verdict-1)
  [ -n "$v2" ]          || missing+=(Lane-7-Verdict-2)
  [ -n "$l7_mergedby" ] || missing+=(Lane-7-Merged-By)
  if [ ${#missing[@]} -gt 0 ]; then
    add_defect d1 "attestation block incomplete, missing: ${missing[*]}"
  fi

  # --- d2: Lane-7-Head is not the commit actually merged ------------------
  if [ -z "$l7_head" ]; then
    : # already reported by d1
  elif ! [[ "$l7_head" =~ ^[0-9a-fA-F]{7,40}$ ]]; then
    add_defect d2 "Lane-7-Head is not a commit sha: '$l7_head'"
  else
    case "$head_known" in
      yes)
        if ! sha_eq "$l7_head" "$merged_head"; then
          add_defect d2 "Lane-7-Head $l7_head is not the commit merged ($(short "$merged_head"))"
        fi ;;
      squash)
        add_uncheck "d2: squash merge — git carries no independent record of the merged head; fetch refs/pull/*/head to check it" ;;
      *)
        add_uncheck "d2: the commit actually merged was not supplied" ;;
    esac
  fi

  # --- d3: the verdict lines ---------------------------------------------
  check_verdict 1 "$v1" "$l7_head"
  check_verdict 2 "$v2" "$l7_head"
  if [ ${#REVIEWERS[@]} -eq 2 ] && [ "${REVIEWERS[0]}" = "${REVIEWERS[1]}" ]; then
    add_advisory "both verdicts name the same reviewer, '${REVIEWERS[0]}' — ADR-0005 §6.2 corollary 2 forbids it"
  fi

  # --- advisories ---------------------------------------------------------
  case "$l7_gate" in
    ''|PASS) ;;
    n/a) add_advisory "Lane-7-Gate reads a bare 'n/a' with no reason; §6.3 requires the 'n/a (<reason>)' form" ;;
    n/a\ \(*\))
      if [ "$has_workflow" = yes ]; then
        add_advisory "Lane-7-Gate is 'n/a' but the lane-gate workflow exists at this commit; §6.3 calls that the gate defect"
      fi ;;
    *) add_advisory "Lane-7-Gate is neither PASS nor 'n/a (<reason>)': '$l7_gate'" ;;
  esac
  case "$l7_ledger" in
    ''|PASS|n/a\ \(*\)) ;;
    n/a) add_advisory "Lane-7-Ledger reads a bare 'n/a' with no reason; §6.3 requires the 'n/a (<reason>)' form" ;;
    *) add_advisory "Lane-7-Ledger is neither PASS nor 'n/a (<reason>)': '$l7_ledger'" ;;
  esac
  if [ -n "$l7_mergedby" ] && [ "$l7_mergedby" != Crucible ]; then
    add_advisory "Lane-7-Merged-By is '$l7_mergedby', not Crucible"
  fi
}

# check_verdict <1|2> <line-body> <lane-7-head>
check_verdict() {
  local idx="$1" body="$2" head="$3"
  if [ -z "$body" ]; then return 0; fi   # absent: d1 reported it

  local covers='' front="$body"
  if [[ "$front" =~ [[:space:]]covers[[:space:]]+([0-9a-fA-F]{7,40})[[:space:]]*$ ]]; then
    covers="${BASH_REMATCH[1]}"
    front="${front:0:${#front}-${#BASH_REMATCH[0]}}"
  else
    add_defect d3 "verdict-$idx states no 'covers <sha>': '$body'"
  fi

  local disposition='' reviewer='' issue=''
  if [[ "$front" =~ ^(APPROVE|REQUEST[[:space:]]+CHANGES)[[:space:]]+([^[:space:]]+)[[:space:]]+(.+)$ ]]; then
    disposition="${BASH_REMATCH[1]}"
    reviewer="${BASH_REMATCH[2]}"
    issue="$(trim "${BASH_REMATCH[3]}")"
    REVIEWERS+=("$reviewer")
  else
    add_defect d3 "verdict-$idx is not '<APPROVE|REQUEST CHANGES> <reviewer> <issue> covers <sha>': '$body'"
    return 0
  fi

  if [ "$disposition" != APPROVE ]; then
    add_advisory "verdict-$idx reads '$disposition', and §6.3 condition 3 merges only on two APPROVEs"
  fi

  # d3, first half: covers <sha> must be Lane-7-Head. Internal to the block, so
  # it is checked with no credential.
  if [ -n "$covers" ] && [ -n "$head" ] && ! sha_eq "$covers" "$head"; then
    add_defect d3 "verdict-$idx covers $covers, which is not Lane-7-Head ($(short "$head"))"
  fi

  # d3, second half: the cited issue must exist and carry that verdict.
  local ident=''
  if [[ "$issue" =~ ([A-Za-z][A-Za-z0-9]*-[0-9]+) ]]; then ident="${BASH_REMATCH[1]}"; fi
  if [ -z "$ident" ]; then
    add_defect d3 "verdict-$idx cites no resolvable Paperclip issue: '$issue'"
    return 0
  fi
  if ! api_ready; then
    add_uncheck "d3: $ident carries no checked verdict — $(api_reason)"
    return 0
  fi
  local result; result="$(verdict_lookup "$ident" "$disposition" "${covers:-$head}")"
  case "$result" in
    found)           ;;
    missing-issue)   add_defect d3 "verdict-$idx cites $ident, which does not exist" ;;
    missing-verdict) add_defect d3 "verdict-$idx cites $ident, where no $disposition covering $(short "${covers:-$head}") is recorded" ;;
    unchecked:*)     add_uncheck "d3: $ident carries no checked verdict — ${result#unchecked:}" ;;
    *)               add_uncheck "d3: $ident carries no checked verdict — unrecognised lookup result" ;;
  esac
}

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

trim() { local s="$1"; s="${s#"${s%%[![:space:]]*}"}"; printf '%s' "${s%"${s##*[![:space:]]}"}"; }
short() { printf '%s' "${1:0:7}"; }

# Two shas match when one is a prefix of the other, so a short sha in the block
# compares against a full one from git.
sha_eq() {
  local a b; a="$(printf '%s' "$1" | tr 'A-F' 'a-f')"; b="$(printf '%s' "$2" | tr 'A-F' 'a-f')"
  [ -n "$a" ] && [ -n "$b" ] && { [ "${a#"$b"}" != "$a" ] || [ "${b#"$a"}" != "$b" ]; }
}

# ---------------------------------------------------------------------------
# Reporting one commit
# ---------------------------------------------------------------------------

n_ok=0; n_defect=0; n_uncheck=0; n_advisory=0
n_d1=0; n_d2=0; n_d3=0; n_recorded=0
IGNORE_RECORDED=''; STRICT=''
EXIT_DEFECT=''; EXIT_UNCHECK=''

report_commit() {
  local commit="$1" subject="$2" recorded="$3"
  local codes='' seen_d1='' seen_d2='' seen_d3='' f

  for f in ${FINDINGS[@]+"${FINDINGS[@]}"}; do
    case "${f%%|*}" in d1) seen_d1=1 ;; d2) seen_d2=1 ;; d3) seen_d3=1 ;; esac
  done
  [ -n "$seen_d1" ] && codes="${codes}d1,"
  [ -n "$seen_d2" ] && codes="${codes}d2,"
  [ -n "$seen_d3" ] && codes="${codes}d3,"
  codes="${codes%,}"

  local class colour
  if [ -n "$codes" ]; then
    class=DEFECT; colour="$C_RED"
    n_defect=$((n_defect+1))
    [ -n "$seen_d1" ] && n_d1=$((n_d1+1))
    [ -n "$seen_d2" ] && n_d2=$((n_d2+1))
    [ -n "$seen_d3" ] && n_d3=$((n_d3+1))
    if [ -n "$recorded" ] && [ -n "$IGNORE_RECORDED" ]; then :; else EXIT_DEFECT=1; fi
  elif [ ${#UNCHECKS[@]} -gt 0 ]; then
    class=UNCHECKED; colour="$C_YEL"; codes='-'
    n_uncheck=$((n_uncheck+1)); EXIT_UNCHECK=1
  else
    class=OK; colour="$C_GRN"; codes='-'
    n_ok=$((n_ok+1))
  fi
  [ ${#ADVISORIES[@]} -gt 0 ] && n_advisory=$((n_advisory+${#ADVISORIES[@]}))
  [ -n "$recorded" ] && n_recorded=$((n_recorded+1))

  printf '%s%-9s%s %-8s %-8s %s\n' "$colour" "$class" "$C_OFF" "$(short "$commit")" "$codes" "$subject"
  [ -n "$recorded" ] && printf '          recorded: %s\n' "$recorded"
  for f in ${FINDINGS[@]+"${FINDINGS[@]}"}; do
    printf '          %s: %s\n' "${f%%|*}" "${f#*|}"
  done
  for f in ${UNCHECKS[@]+"${UNCHECKS[@]}"}; do
    printf '          UNCHECKED %s\n' "$f"
  done
  for f in ${ADVISORIES[@]+"${ADVISORIES[@]}"}; do
    printf '          advisory: %s\n' "$f"
  done
}

print_footer() {
  local scanned="$1" skipped="$2" first="$3"
  printf '\n'
  if [ -n "$first" ]; then
    printf 'Scanned %s commit(s) on the first-parent history from %s, the first attested merge.\n' \
      "$scanned" "$(short "$first")"
  else
    printf 'Scanned %s commit(s).\n' "$scanned"
  fi
  [ "$skipped" -gt 0 ] && printf '%s earlier commit(s) skipped: pre-attestation history (ADR-0005 §7 item 10).\n' "$skipped"
  printf '  OK         %s\n' "$n_ok"
  printf '  DEFECT     %s   (d1: %s, d2: %s, d3: %s)\n' "$n_defect" "$n_d1" "$n_d2" "$n_d3"
  printf '  UNCHECKED  %s\n' "$n_uncheck"
  printf '  advisory   %s   (outside the three §6.3 defects; does not affect the exit status)\n' "$n_advisory"
  [ "$n_recorded" -gt 0 ] && printf '  of the above, %s commit(s) are recorded defects from ADR-0005 §6.4\n' "$n_recorded"
  cat <<'EOF'

Not evidenced by the block, and so not checked here: §6.3 condition 3's "zero unresolved
blocking findings" clause cannot be reconstructed from `main`. OK means the attestation is
well formed and internally consistent — not that the review it attests to was sound.
EOF
}

finish() {
  if [ -n "$EXIT_DEFECT" ]; then exit 1; fi
  if [ -n "$STRICT" ] && [ -n "$EXIT_UNCHECK" ]; then exit 1; fi
  exit 0
}

# ---------------------------------------------------------------------------
# scan
# ---------------------------------------------------------------------------

cmd_scan() {
  local ref='' repo='.'
  while [ $# -gt 0 ]; do
    case "$1" in
      --repo) repo="${2:?--repo needs a directory}"; shift 2 ;;
      --strict) STRICT=1; shift ;;
      --ignore-recorded) IGNORE_RECORDED=1; shift ;;
      -*) printf 'unknown option: %s\n' "$1" >&2; exit 2 ;;
      *) ref="$1"; shift ;;
    esac
  done
  cd "$repo" || exit 2
  ref="${ref:-origin/main}"
  git rev-parse --verify --quiet "$ref^{commit}" >/dev/null || {
    printf 'not a commit in %s: %s\n' "$repo" "$ref" >&2; exit 2; }

  local commits=() c
  while IFS= read -r c; do [ -n "$c" ] && commits+=("$c"); done \
    < <(git rev-list --first-parent --reverse "$ref")
  if [ ${#commits[@]} -eq 0 ]; then
    printf 'no commits on %s\n' "$ref"; print_footer 0 0 ''; finish
  fi

  # Pass 1 — locate the first attested merge.
  local first_idx=-1 i=0
  for c in "${commits[@]}"; do
    if git log -1 --format=%B "$c" | grep -q '^Lane-7-Head:'; then first_idx=$i; break; fi
    i=$((i+1))
  done

  printf '%-9s %-8s %-8s %s\n' CLASS COMMIT DEFECTS SUBJECT
  printf '%-9s %-8s %-8s %s\n' --------- -------- -------- -------------------------------

  local scanned=0 skipped=0 first_sha=''
  [ "$first_idx" -ge 0 ] && first_sha="${commits[$first_idx]}"

  i=0
  for c in "${commits[@]}"; do
    local recorded; recorded="$(known_defective_note "$c")"
    if { [ "$first_idx" -lt 0 ] || [ "$i" -lt "$first_idx" ]; } && [ -z "$recorded" ]; then
      skipped=$((skipped+1)); i=$((i+1)); continue
    fi

    local subject parents merged_head head_known='' has_workflow=no
    subject="$(git log -1 --format=%s "$c")"
    parents="$(git log -1 --format=%P "$c")"
    # shellcheck disable=SC2086
    set -- $parents
    if [ "$#" -ge 2 ]; then
      merged_head="$2"; head_known=yes
    else
      merged_head=''; head_known=squash
      # A squash leaves `(#N)` in the subject. If refs/pull/N/head has been
      # fetched, git can still say what was merged.
      if [[ "$subject" =~ \(#([0-9]+)\)[[:space:]]*$ ]]; then
        local pr="${BASH_REMATCH[1]}" prsha
        prsha="$(git rev-parse --verify --quiet "refs/pull/$pr/head" || true)"
        if [ -n "$prsha" ]; then merged_head="$prsha"; head_known=yes; fi
      fi
    fi
    git cat-file -e "$c:.github/workflows/lane-gate.yml" 2>/dev/null && has_workflow=yes || has_workflow=no

    # A here-string, not a pipe: a pipeline would run classify_message in a
    # subshell and its findings would never reach report_commit.
    classify_message "$c" "$merged_head" "$head_known" "$has_workflow" \
      <<< "$(git log -1 --format=%B "$c")"
    report_commit "$c" "$subject" "$recorded"
    scanned=$((scanned+1)); i=$((i+1))
  done

  print_footer "$scanned" "$skipped" "$first_sha"
  finish
}

# ---------------------------------------------------------------------------
# classify — the git-free, credential-free fixture surface
# ---------------------------------------------------------------------------

cmd_classify() {
  local commit='(stdin)' merged_head='' subject='' has_workflow=unknown head_known=''
  while [ $# -gt 0 ]; do
    case "$1" in
      --commit) commit="${2:?--commit needs a sha}"; shift 2 ;;
      --merged-head) merged_head="${2:?--merged-head needs a sha}"; head_known=yes; shift 2 ;;
      --squash) head_known=squash; shift ;;
      --subject) subject="${2-}"; shift 2 ;;
      --has-workflow) has_workflow="${2:?--has-workflow needs yes or no}"; shift 2 ;;
      --strict) STRICT=1; shift ;;
      --ignore-recorded) IGNORE_RECORDED=1; shift ;;
      -*) printf 'unknown option: %s\n' "$1" >&2; exit 2 ;;
      *) printf 'unexpected argument: %s\n' "$1" >&2; exit 2 ;;
    esac
  done
  classify_message "$commit" "$merged_head" "${head_known:-unknown}" "$has_workflow"
  report_commit "$commit" "$subject" "$(known_defective_note "$commit")"
  finish
}

# ---------------------------------------------------------------------------

case "${1:-help}" in
  scan)     shift; cmd_scan "$@" ;;
  classify) shift; cmd_classify "$@" ;;
  help|-h|--help) usage 0 ;;
  *) printf 'usage: %s {scan|classify|help} [options]\n' "$0" >&2; exit 2 ;;
esac
