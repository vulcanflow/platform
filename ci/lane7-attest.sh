#!/usr/bin/env bash
# lane7-attest.sh — the lane-7 attestation detector (ADR-0005 §6.3 condition 4).
#
# ADR-0005 §3.1 means one GitHub identity serves all nine agents, so nothing in
# GitHub distinguishes a Crucible merge from any other and lane 7 has no
# mechanical backstop. §6.3 condition 4 answers that by requiring every merge to
# a default branch to carry an attestation block in its commit message — which
# does not prevent an unattested merge, but makes one visible from `main`'s own
# history afterwards. §10 R5b stays live until something reads it. This is that
# reader.
#
#   ci/lane7-attest.sh commit <owner>/<repo> <sha>   one commit, from the cwd
#   ci/lane7-attest.sh repo   <owner>/<repo>         the manifest range, from the cwd
#   ci/lane7-attest.sh run    [<owner>/<repo> ...]   clone and scan; the production entry
#
# `commit` and `repo` read git facts from the current working directory and clone
# nothing, which is what lets ci/lane7-attest-test.sh build a throwaway
# repository under `mktemp -d` and assert a verdict with no network.
#
# stdout is the ledger and nothing else; every diagnostic goes to stderr. Four
# line shapes, and nothing else on stdout begins `MODE `, `RANGE `, `OK ` or
# `L7-`:
#
#   MODE live | MODE fixture <dir>                always, the first stdout line
#   RANGE <owner>/<repo> <count>                 once per repository classified
#   OK <owner>/<repo> <sha40>                    once per clean commit
#   L7-<CODE> <owner>/<repo> <sha40> -- <prose>  once per finding
#
# The three fields before ` -- ` are the contract. Everything after it is prose
# for a human: a reworded message can never change a verdict. Each
# (code, repo, sha) triple is emitted at most once per commit, so a code two
# ADR-0005 §6.5 classes both reach is one finding and one line.
#
# Exit: 0 ledger complete, no non-advisory finding; 1 ledger complete with at
# least one; 2 the run did not complete — usage error, bad manifest, not a git
# repository. The advisory set is exactly {L7-LEDGER-VACUOUS}. Every UNCHECKED
# code is non-advisory and exits 1, because a check that passes when it could not
# run is the shape of §6.2 corollary 4 and of §6.4 itself.
#
# The interface above is fixed by plans/lane7-attest-detector-spec.md revision 3
# (docs#35), which is lane 1's; the 46 fixtures that assert it are
# ci/lane7-attest-test.sh, which is lane 2's. Neither is this file's to change.

set -euo pipefail

FLOOR_MANIFEST="$(dirname "$0")/lane7-attest-floors.txt"

note() { printf '%s\n' "$*" >&2; }
die2() { printf 'lane7-attest: %s\n' "$*" >&2; exit 2; }

usage() {
  sed -n '2,/^$/p' "$0" | sed 's/^#\{1,\} \{0,1\}//' >&2
  exit "${1:-2}"
}

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

trim() { local s="${1-}"; s="${s#"${s%%[![:space:]]*}"}"; printf '%s' "${s%"${s##*[![:space:]]}"}"; }
lower() { printf '%s' "${1-}" | tr 'A-Z' 'a-z'; }
short() { printf '%s' "${1:0:7}"; }

is_sha_shaped() { [[ "${1-}" =~ ^[0-9a-f]{7,40}$ ]]; }

# Two shas match when one is a prefix of the other, so a short sha in the block
# compares against a full one from git.
sha_eq() {
  local a b; a="$(lower "${1-}")"; b="$(lower "${2-}")"
  [ -n "$a" ] && [ -n "$b" ] && { [ "${a#"$b"}" != "$a" ] || [ "${b#"$a"}" != "$b" ]; }
}

# ---------------------------------------------------------------------------
# Mode and credentials
#
# LANE7_FIXTURE_DIR is a test seam in production code, and the spec's §6.3 is
# explicit about what holds it down rather than pretending it is free: the ledger
# records which mode produced it, unconditionally and first, so a fixture-mode
# ledger offered as evidence for R5b's closure is self-identifying.
# ---------------------------------------------------------------------------

FIXTURE_DIR="${LANE7_FIXTURE_DIR:-}"
MODE_EMITTED=''

emit_mode() {
  if [ -n "$MODE_EMITTED" ]; then return 0; fi
  MODE_EMITTED=1
  if [ -n "$FIXTURE_DIR" ]; then
    printf 'MODE fixture %s\n' "$FIXTURE_DIR"
  else
    printf 'MODE live\n'
  fi
}

# An absent or empty credential is never a usage error. It degrades the classes
# that credential gates, reports the corresponding *-UNCHECKED code per commit,
# and exits 1 — spec §6.2, which is what keeps fixtures 24, 45 and 46
# satisfiable. Exit 2 stays reserved for "the detector did not complete".
have_gh_token() { [ -n "${LANE7_GITHUB_TOKEN:-}" ]; }
have_pc_token() { [ -n "${LANE7_PAPERCLIP_TOKEN:-}" ]; }

# ---------------------------------------------------------------------------
# The floor manifest — spec §2
#
# Resolved against the script, never against the working directory and never
# from an environment variable: a floor that can be redirected at run time is
# the thing ADR-0005 §6.5's "moving a floor is an amendment to this record"
# exists to prevent.
# ---------------------------------------------------------------------------

declare -A FLOOR=()
MANIFEST_ORDER=()
MANIFEST_LOADED=''

load_manifest() {
  if [ -n "$MANIFEST_LOADED" ]; then return 0; fi
  [ -r "$FLOOR_MANIFEST" ] || die2 "cannot read the floor manifest at $FLOOR_MANIFEST"

  local lineno=0 line repo sha
  while IFS= read -r line || [ -n "$line" ]; do
    lineno=$((lineno + 1))
    case "$(trim "$line")" in ''|'#'*) continue ;; esac
    # shellcheck disable=SC2086
    set -- $line
    [ "$#" -eq 2 ] || die2 "$FLOOR_MANIFEST:$lineno: expected two fields, found $#"
    repo="$1"; sha="$2"
    [[ "$repo" =~ ^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$ ]] \
      || die2 "$FLOOR_MANIFEST:$lineno: '$repo' is not <owner>/<repo>"
    [[ "$sha" =~ ^[0-9a-f]{40}$ ]] \
      || die2 "$FLOOR_MANIFEST:$lineno: '$sha' is not 40 lowercase hex — a manifest is a record, and a record is unabbreviated"
    [ -z "${FLOOR[$repo]+x}" ] || die2 "$FLOOR_MANIFEST:$lineno: '$repo' appears twice"
    FLOOR["$repo"]="$sha"
    MANIFEST_ORDER+=("$repo")
  done < "$FLOOR_MANIFEST"

  MANIFEST_LOADED=1
}

# ---------------------------------------------------------------------------
# Output
# ---------------------------------------------------------------------------

EXIT_FINDING=''
ADVISORY_CODES=' L7-LEDGER-VACUOUS '   # spec §5: the advisory set, exactly
EMITTED=''                             # de-duplication, reset per commit

is_advisory() { case "$ADVISORY_CODES" in *" $1 "*) return 0 ;; *) return 1 ;; esac; }

# finding <code> <repo> <sha-or-dash> <prose...>
finding() {
  local code="$1" repo="$2" sha="$3"; shift 3
  case "$EMITTED" in *" $code "*) return 0 ;; esac
  EMITTED="$EMITTED $code "
  printf '%s %s %s -- %s\n' "$code" "$repo" "$sha" "$*"
  if ! is_advisory "$code"; then EXIT_FINDING=1; fi
}

# ---------------------------------------------------------------------------
# The three non-git fact sources
#
# Each writes its answer to stdout and returns 0, or returns non-zero meaning
# "the lookup could not run". The difference between those two is the whole of
# §6.2 corollary 4, so it is a return code and never a value.
#
# Under LANE7_FIXTURE_DIR no network call of any kind is made and every non-git
# fact comes from a flat file keyed by sha or by issue. An absent file means the
# lookup failed; a present-but-empty file means the lookup ran and the answer was
# nothing. That distinction is load-bearing in two places (spec §6.1).
# ---------------------------------------------------------------------------

gh_api() {
  curl -sS -f --max-time 20 \
    -H "Authorization: Bearer ${LANE7_GITHUB_TOKEN:-}" \
    -H 'Accept: application/vnd.github+json' \
    -H 'X-GitHub-Api-Version: 2022-11-28' \
    "https://api.github.com$1"
}

# lookup_pr <repo> <sha> -> the pull-request number, or the literal `none`
#
# The route is GET /repos/{owner}/{repo}/commits/{sha}/pulls and only that.
# Parsing a `(#n)` suffix out of the commit subject is not the route and must not
# be the fallback: ADR-0005 §7 item 10 requires the merge message to be typed, so
# GitHub's auto-appended suffix is not guaranteed to survive, and a detector that
# depends on it fires the most serious code in the set on a legitimate merge.
lookup_pr() {
  local repo="$1" sha="$2" json
  have_gh_token || { note "no LANE7_GITHUB_TOKEN: the pull-request lookup for $(short "$sha") did not run"; return 1; }
  if [ -n "$FIXTURE_DIR" ]; then
    [ -f "$FIXTURE_DIR/pulls/$sha" ] || return 1
    trim "$(head -n 1 "$FIXTURE_DIR/pulls/$sha")"
    return 0
  fi
  json="$(gh_api "/repos/$repo/commits/$sha/pulls")" || return 1
  PC_JSON="$json" python3 -c '
import json, os
rows = json.loads(os.environ["PC_JSON"])
print(rows[0]["number"] if rows else "none")
' 2>/dev/null || return 1
}

# lookup_pr_head <repo> <n> -> the sha refs/pull/<n>/head resolves to
lookup_pr_head() {
  local repo="$1" n="$2" sha
  have_gh_token || { note "no LANE7_GITHUB_TOKEN: refs/pull/$n/head was not resolved"; return 1; }
  if [ -n "$FIXTURE_DIR" ]; then
    [ -f "$FIXTURE_DIR/pull-head/$n" ] || return 1
    trim "$(head -n 1 "$FIXTURE_DIR/pull-head/$n")"
    return 0
  fi
  # refs/pull/<n>/head literally, because GitHub retains it after the branch is
  # deleted and §6.4's branch was deleted at merge. Local first, so a `run` scan
  # over a repository whose pull refs were fetched makes no call per commit.
  sha="$(git rev-parse --verify --quiet "refs/pull/$n/head" || true)"
  if [ -z "$sha" ]; then
    sha="$(git ls-remote "https://github.com/$repo.git" "refs/pull/$n/head" 2>/dev/null | cut -f1)" || return 1
  fi
  [ -n "$sha" ] || return 1
  printf '%s' "$sha"
}

# lookup_check_runs <repo> <sha> -> zero or more `<name> <conclusion>` lines
lookup_check_runs() {
  local repo="$1" sha="$2" json
  have_gh_token || { note "no LANE7_GITHUB_TOKEN: the check-run lookup for $(short "$sha") did not run"; return 1; }
  if [ -n "$FIXTURE_DIR" ]; then
    [ -f "$FIXTURE_DIR/check-runs/$sha" ] || return 1
    cat "$FIXTURE_DIR/check-runs/$sha"
    return 0
  fi
  json="$(gh_api "/repos/$repo/commits/$sha/check-runs?per_page=100")" || return 1
  PC_JSON="$json" python3 -c '
import json, os
for r in json.loads(os.environ["PC_JSON"]).get("check_runs", []):
    print(r.get("name", "?"), r.get("conclusion") or "pending")
' 2>/dev/null || return 1
}

# lookup_verdicts <VUL-n> -> zero or more `<reviewer> <disposition> <sha>` records
#
# Return 0 the lookup ran; 3 the issue does not exist; anything else the lookup
# could not run. Three outcomes and not two, because §4.1 makes "the cited issue
# is not there" a transcription defect in the attestation and "the issue is there
# and records nothing" a lane-6/lane-7 defect, with two different repairs.
lookup_verdicts() {
  local ident="$1" base company
  have_pc_token || { note "no LANE7_PAPERCLIP_TOKEN: the verdict contents of $ident were not checked"; return 1; }
  if [ -n "$FIXTURE_DIR" ]; then
    [ -f "$FIXTURE_DIR/verdicts/$ident" ] || return 3
    cat "$FIXTURE_DIR/verdicts/$ident"
    return 0
  fi
  base="${LANE7_PAPERCLIP_URL:-${PAPERCLIP_API_URL:-}}"
  company="${LANE7_PAPERCLIP_COMPANY:-${PAPERCLIP_COMPANY_ID:-}}"
  if [ -z "$base" ] || [ -z "$company" ]; then
    note "no Paperclip base URL or company id: the verdict contents of $ident were not checked"
    return 1
  fi
  PC_IDENT="$ident" PC_BASE="${base%/}" PC_COMPANY="$company" \
  PC_TOKEN="$LANE7_PAPERCLIP_TOKEN" python3 -c '
import json, os, re, sys, urllib.parse, urllib.request

base = os.environ["PC_BASE"]
base = base[:-4] if base.endswith("/api") else base
ident = os.environ["PC_IDENT"]


def get(path):
    req = urllib.request.Request(
        base + path, headers={"Authorization": "Bearer " + os.environ["PC_TOKEN"]})
    with urllib.request.urlopen(req, timeout=20) as r:
        return json.load(r)


def rows(doc, *keys):
    if isinstance(doc, list):
        return doc
    for k in keys:
        if isinstance(doc.get(k), list):
            return doc[k]
    return []


try:
    found = get("/api/companies/%s/issues?q=%s"
                % (os.environ["PC_COMPANY"], urllib.parse.quote(ident)))
except Exception:
    sys.exit(1)

issue = next((i for i in rows(found, "issues", "data", "items")
              if str(i.get("identifier", "")).upper() == ident.upper()), None)
if issue is None:
    sys.exit(3)

try:
    bodies = [str(issue.get("description") or "")]
    bodies += [str(c.get("body") or "") for c in
               rows(get("/api/issues/%s/comments" % issue["id"]), "comments", "data", "items")]
except Exception:
    sys.exit(1)

# §4.2 rule 1: a verdict at the issue is ONE record. The reviewer, the
# disposition and the covered sha must belong to one parsed record. Co-occurrence
# anywhere in a body is explicitly not a match, because that is exactly what let
# a REQUEST CHANGES at VUL-31 satisfy an APPROVE citation — blocking finding B1
# against the withdrawn head of platform#6.
RECORD = re.compile(
    r"^[\s>*\-]*(?:Lane-6-)?Verdict\s*:\s*"
    r"(APPROVE|REQUEST[_ ]CHANGES)\s+"
    r"([A-Za-z][A-Za-z0-9_-]*)\b[^\n]*?\bcovers\s+([0-9a-fA-F]{7,40})",
    re.IGNORECASE | re.MULTILINE)

out = ["%s %s %s" % (m.group(2), m.group(1).upper().replace(" ", "_"), m.group(3).lower())
       for body in bodies for m in RECORD.finditer(body)]
if not out:
    # The issue exists and carries no record this parser can read. That is "I
    # could not look", not "the verdict is absent" — reporting the second from
    # the first is the fail-open this detector exists to catch.
    sys.stderr.write("no machine-readable verdict record at %s\n" % ident)
    sys.exit(1)
print("\n".join(out))
'
}

# ---------------------------------------------------------------------------
# The attestation block
# ---------------------------------------------------------------------------

BLOCK_PRESENT=''
K_HEAD=''; K_GATE=''; K_LEDGER=''; K_MERGEDBY=''
MISSING_KEYS=()
VERDICT_BODIES=()
TRAILING_OFFENCE=''

parse_block() {
  local msg="$1" line i last=-1
  local -a lines=()
  mapfile -t lines <<< "$msg"

  BLOCK_PRESENT=''; K_HEAD=''; K_GATE=''; K_LEDGER=''; K_MERGEDBY=''
  MISSING_KEYS=(); VERDICT_BODIES=(); TRAILING_OFFENCE=''

  for i in "${!lines[@]}"; do
    line="${lines[$i]}"
    case "$line" in Lane-7-*) BLOCK_PRESENT=1; last="$i" ;; esac
    case "$line" in
      Lane-7-Head:*)      K_HEAD="$(trim "${line#Lane-7-Head:}")" ;;
      Lane-7-Gate:*)      K_GATE="$(trim "${line#Lane-7-Gate:}")" ;;
      Lane-7-Ledger:*)    K_LEDGER="$(trim "${line#Lane-7-Ledger:}")" ;;
      Lane-7-Merged-By:*) K_MERGEDBY="$(trim "${line#Lane-7-Merged-By:}")" ;;
      Lane-7-Verdict-*:*) VERDICT_BODIES+=("$(trim "${line#*:}")") ;;
      Lane-7-*)           note "unrecognised attestation line: ${line%%:*}" ;;
    esac
  done

  if [ -z "$BLOCK_PRESENT" ]; then return 0; fi

  [ -n "$K_HEAD" ]     || MISSING_KEYS+=(Lane-7-Head)
  [ -n "$K_GATE" ]     || MISSING_KEYS+=(Lane-7-Gate)
  [ -n "$K_LEDGER" ]   || MISSING_KEYS+=(Lane-7-Ledger)
  [ -n "$K_MERGEDBY" ] || MISSING_KEYS+=(Lane-7-Merged-By)

  # §7: after the block, only blank lines, a run of three or more `-` (GitHub's
  # squash-UI separator) and trailer lines may appear. Read strictly, the first
  # correct attested merge made through the web UI would carry L7-NOT-TRAILING,
  # and a detector that fires on the behaviour it exists to bless gets switched
  # off. Read loosely — "anything may follow" — the block stops being the end of
  # the message and a second, contradictory block can be appended below it.
  for ((i = last + 1; i < ${#lines[@]}; i++)); do
    line="${lines[$i]}"
    if [ -z "$(trim "$line")" ]; then continue; fi
    if [[ "$line" =~ ^-{3,}[[:space:]]*$ ]]; then continue; fi
    if [[ "$line" =~ ^[A-Za-z][A-Za-z0-9-]*:[[:space:]] ]]; then continue; fi
    TRAILING_OFFENCE="$line"
    break
  done
}

# parse_verdict <body> — splits one Lane-7-Verdict-* line into V_DISP, V_WHO,
# V_ISSUE and V_COVERS. `REQUEST CHANGES` is two whitespace-separated words, so
# the disposition is not simply field 1.
V_DISP=''; V_WHO=''; V_ISSUE=''; V_COVERS=''; V_COVERS_SEEN=''
parse_verdict() {
  local front="${1-}"
  V_DISP=''; V_WHO=''; V_ISSUE=''; V_COVERS=''; V_COVERS_SEEN=''

  if [[ "$front" =~ [[:space:]]covers[[:space:]]+([^[:space:]]+)[[:space:]]*$ ]]; then
    V_COVERS="${BASH_REMATCH[1]}"
    V_COVERS_SEEN=1
    front="${front:0:${#front} - ${#BASH_REMATCH[0]}}"
  fi

  # shellcheck disable=SC2086
  set -- $front
  if [ "${1:-}" = REQUEST ] && { [ "${2:-}" = CHANGES ] || [ "${2:-}" = CHANGES: ]; }; then
    V_DISP='REQUEST CHANGES'; V_WHO="${3:-}"
    [ "$#" -ge 3 ] && shift 3 || shift "$#"
  else
    V_DISP="${1:-}"; V_WHO="${2:-}"
    [ "$#" -ge 2 ] && shift 2 || shift "$#"
  fi
  V_ISSUE="$*"
}

# ---------------------------------------------------------------------------
# classify_commit <owner>/<repo> <sha40>
#
# Rules A, B and C of the spec's §4.3 are an application order over ADR-0005
# §6.5's eight classes. They add no class and no code:
#
#   A  a check does not run on a value it does not have. An absent key fires
#      L7-KEY and suppresses everything that reads it — and, inside a verdict
#      line, a malformed disposition or `covers` sha suppresses the four
#      verdict-contents lookups.
#   B  class 3 is suppressed only by its own lookup failing.
#   C  a wrong value is present, so it is used. Suppressing class 8 whenever
#      class 2 fires would let a commit attest a false PASS *and* a false Head
#      with the first going unreported, which is the §6.4 shape twice over.
# ---------------------------------------------------------------------------

classify_commit() {
  local repo="$1" sha="$2"
  EMITTED=''

  local msg parents has_workflow=no touches_crates=no
  msg="$(git log -1 --format=%B "$sha")"
  parents="$(git log -1 --format=%P "$sha")"
  if git cat-file -e "$sha:.github/workflows/lane-gate.yml" 2>/dev/null; then has_workflow=yes; fi
  if commit_touches_crates "$sha"; then touches_crates=yes; fi

  # shellcheck disable=SC2086
  set -- $parents
  local nparents="$#" second_parent="${2:-}"

  parse_block "$msg"

  # --- class 3, first, because rule B makes it independent of the block -----
  local pr='' pr_ok=''
  if pr="$(lookup_pr "$repo" "$sha")"; then pr_ok=1; fi
  if [ -n "$pr_ok" ] && [ "$pr" != none ] && ! [[ "$pr" =~ ^[0-9]+$ ]]; then
    note "the pull-request lookup for $(short "$sha") answered '$pr', which is neither a number nor 'none'"
    pr_ok=''
  fi
  if [ -z "$pr_ok" ]; then
    finding L7-PR-UNCHECKED "$repo" "$sha" \
      'the pull-request association lookup could not run; "could not resolve" is not "no pull request"'
  elif [ "$pr" = none ]; then
    finding L7-NOT-PR "$repo" "$sha" \
      'reached main by neither a merge nor a squash of a pull request'
  fi

  # --- class 1 -------------------------------------------------------------
  if [ -z "$BLOCK_PRESENT" ]; then
    finding L7-MISSING "$repo" "$sha" 'no attestation block (ADR-0005 §6.3 condition 4)'
    return 0   # §6.5: class 1 suppresses classes 2 and 4-8; class 3 ran above
  fi

  if [ "${#MISSING_KEYS[@]}" -gt 0 ]; then
    finding L7-KEY "$repo" "$sha" "attestation key(s) absent: ${MISSING_KEYS[*]}"
  fi
  if [ -n "$TRAILING_OFFENCE" ]; then
    finding L7-NOT-TRAILING "$repo" "$sha" \
      "the block does not end the message; first offending line: ${TRAILING_OFFENCE:0:60}"
  fi

  # --- class 2 -------------------------------------------------------------
  #
  # head_usable gates every later check keyed on Lane-7-Head: class 8, the
  # verdict-contents lookups, and L7-VERDICT-COVERS. Rule A, and §4.2 rule 2 —
  # no lookup is ever performed with an empty sha, because `"" in body` is
  # vacuously true and that vacuity is the live false pass this detector exists
  # to have caught.
  local head_usable=''
  if [ -z "$K_HEAD" ]; then
    :                                   # L7-KEY said it; rule A suppresses the rest
  elif ! is_sha_shaped "$(lower "$K_HEAD")"; then
    finding L7-HEAD "$repo" "$sha" "Lane-7-Head is not a commit sha: '$K_HEAD'"
  elif [ "$nparents" -ge 2 ]; then
    head_usable=1
    if ! sha_eq "$K_HEAD" "$second_parent"; then
      finding L7-HEAD "$repo" "$sha" \
        "Lane-7-Head $(short "$K_HEAD") is not the commit merged ($(short "$second_parent"))"
    fi
  else
    head_usable=1
    # A squash leaves no second parent, so the commit actually merged is
    # refs/pull/<n>/head. Rule A: with no pull-request number there is nothing to
    # resolve, and rule B has already reported why.
    local prhead
    if [ -z "$pr_ok" ] || [ "$pr" = none ]; then
      :
    elif prhead="$(lookup_pr_head "$repo" "$pr")" && [ -n "$prhead" ]; then
      if ! sha_eq "$K_HEAD" "$prhead"; then
        finding L7-HEAD "$repo" "$sha" \
          "Lane-7-Head $(short "$K_HEAD") is not refs/pull/$pr/head ($(short "$prhead"))"
      fi
    else
      finding L7-HEAD-UNRESOLVABLE "$repo" "$sha" \
        "pull request #$pr was identified but refs/pull/$pr/head does not resolve"
    fi
  fi

  # --- classes 4 and 8 -----------------------------------------------------
  local gate_pass=''
  if [ -n "$K_GATE" ]; then
    case "$K_GATE" in
      PASS) gate_pass=1 ;;
      n/a)
        finding L7-GATE-BARE "$repo" "$sha" \
          "Lane-7-Gate reads a bare 'n/a'; §6.3 requires the 'n/a (<reason>)' form" ;;
      n/a\ \(*\)) ;;
      *)
        finding L7-GATE-VALUE "$repo" "$sha" \
          "Lane-7-Gate is neither PASS nor an 'n/a' form: '$K_GATE'" ;;
    esac
    case "$K_GATE" in
      n/a|n/a\ *)
        if [ "$has_workflow" = yes ]; then
          finding L7-GATE-VACUOUS "$repo" "$sha" \
            "Lane-7-Gate is 'n/a' but this tree carries .github/workflows/lane-gate.yml; §6.3 calls that the gate defect"
        fi ;;
    esac
  fi

  if [ -n "$gate_pass" ] && [ -n "$head_usable" ]; then
    check_gate_confirmed "$repo" "$sha"
  fi

  # --- class 5 -------------------------------------------------------------
  if [ -n "$K_LEDGER" ]; then
    case "$K_LEDGER" in
      PASS) ;;
      n/a)
        finding L7-LEDGER-BARE "$repo" "$sha" \
          "Lane-7-Ledger reads a bare 'n/a'; §6.3 requires the 'n/a (<reason>)' form" ;;
      n/a\ \(*\)) ;;
      *) note "Lane-7-Ledger is neither PASS nor an 'n/a' form: '$K_LEDGER' — no code is enumerated for this" ;;
    esac
    case "$K_LEDGER" in
      n/a|n/a\ *)
        if [ "$touches_crates" = yes ]; then
          # The one advisory code (§5). Whether a change engaged a §25
          # identifier is a spec judgment, so this is reported for lane 6 to
          # adjudicate and must not change the exit code.
          finding L7-LEDGER-VACUOUS "$repo" "$sha" \
            "Lane-7-Ledger is 'n/a' on a commit touching crates/** — advisory, for lane 6"
        fi ;;
    esac
  fi

  # --- classes 6 and 7 -----------------------------------------------------
  classify_verdicts "$repo" "$sha" "$head_usable"

  if [ -n "$K_MERGEDBY" ] && [ "$K_MERGEDBY" != Crucible ]; then
    finding L7-MERGED-BY "$repo" "$sha" "Lane-7-Merged-By is '$K_MERGEDBY', not Crucible"
  fi
}

# check_gate_confirmed <repo> <sha>
#
# Class 8: `Lane-7-Gate: PASS` is true of the commit, not merely typed into it. A
# check run that is absent or still running does not establish that the PASS is
# false — only that nothing confirmed it, which is what the detector can honestly
# say. Hence UNCONFIRMED rather than UNTRUE.
check_gate_confirmed() {
  local repo="$1" sha="$2" runs='' unconfirmed='' name conclusion
  if ! runs="$(lookup_check_runs "$repo" "$K_HEAD")"; then
    finding L7-GATE-UNCHECKED "$repo" "$sha" \
      'the check-run lookup could not run, so the attested PASS was not confirmed'
    return 0
  fi
  if [ -z "$(trim "$runs")" ]; then
    unconfirmed='the commit has no check runs at all'
  else
    while read -r name conclusion _; do
      if [ -z "$name" ]; then continue; fi
      case "$(lower "${conclusion:-pending}")" in
        success) ;;
        *) unconfirmed="required check '$name' is '${conclusion:-pending}'"; break ;;
      esac
    done <<< "$runs"
  fi
  if [ -n "$unconfirmed" ]; then
    finding L7-GATE-UNCONFIRMED "$repo" "$sha" \
      "Lane-7-Gate: PASS is attested and nothing confirms it — $unconfirmed"
  fi
}

# classify_verdicts <repo> <sha> <head-usable>
#
# §4.3: a class-6 code that fires because one of the lookup's inputs is not in
# the shape the lookup needs suppresses all four L7-VERDICT-* lookup codes. The
# suppression is for the commit and not for the one line, because two records of
# one reviewer — or three verdict lines, or a disposition the detector cannot
# confirm — is not the pair the lookup asks about.
#
# Fixture 22 against fixture 37 is the boundary. 22's `covers` is a well-formed
# sha that is simply wrong, so rule C uses it and the lookup still runs, keyed on
# Lane-7-Head; 37's is malformed, so rule A means nothing can be keyed on it.
# Same expected set, two mechanisms, and an implementation that confuses them
# fails exactly one of the two.
classify_verdicts() {
  local repo="$1" sha="$2" head_usable="$3"
  local shape_bad='' i body vid rc records
  local -a who=() ident=()

  if [ "${#VERDICT_BODIES[@]}" -ne 2 ]; then
    finding L7-VERDICT-COUNT "$repo" "$sha" \
      "${#VERDICT_BODIES[@]} Lane-7-Verdict-* line(s); §6.3 condition 4 requires exactly two"
    return 0   # no pair to look up, and no other class-6 code has a well-formed pair either
  fi

  for i in 0 1; do
    body="${VERDICT_BODIES[$i]}"
    parse_verdict "$body"
    who+=("$V_WHO")

    case "$V_WHO" in
      Assay|Warren) ;;
      *) finding L7-VERDICT-WHO "$repo" "$sha" \
           "verdict-$((i + 1)) names '$V_WHO'; §6.2 corollary 2 admits only Assay and Warren"
         shape_bad=1 ;;
    esac

    if [ "$V_DISP" != APPROVE ]; then
      finding L7-VERDICT-DISP "$repo" "$sha" \
        "verdict-$((i + 1))'s disposition is '$V_DISP', not exactly APPROVE"
      shape_bad=1
    fi

    vid=''
    if [[ "$V_ISSUE" =~ (VUL-[0-9]+) ]]; then vid="${BASH_REMATCH[1]}"; fi
    ident+=("$vid")
    if [ -z "$vid" ]; then
      finding L7-VERDICT-ISSUE "$repo" "$sha" "verdict-$((i + 1)) cites no VUL-<n>: '$V_ISSUE'"
      shape_bad=1
    fi

    if [ -z "$V_COVERS_SEEN" ]; then
      finding L7-VERDICT-COVERS "$repo" "$sha" "verdict-$((i + 1)) states no 'covers <sha>'"
      shape_bad=1
    elif ! is_sha_shaped "$V_COVERS"; then
      finding L7-VERDICT-COVERS "$repo" "$sha" \
        "verdict-$((i + 1))'s covers value is not 7-40 lowercase hex: '$V_COVERS'"
      shape_bad=1
    elif [ -n "$head_usable" ] && ! sha_eq "$V_COVERS" "$K_HEAD"; then
      finding L7-VERDICT-COVERS "$repo" "$sha" \
        "verdict-$((i + 1)) covers $(short "$V_COVERS"), which is not Lane-7-Head ($(short "$K_HEAD"))"
    fi
  done

  if [ "${who[0]}" = "${who[1]}" ]; then
    finding L7-VERDICT-DUP "$repo" "$sha" \
      "both verdicts name '${who[0]}'; §6.2 corollary 2 requires two distinct reviewers"
    shape_bad=1
  fi

  if [ -n "$shape_bad" ]; then return 0; fi    # rule A, over the verdict fields
  if [ -z "$head_usable" ]; then return 0; fi  # §4.2 rule 2: never look up on an empty sha

  for i in 0 1; do
    rc=0
    records="$(lookup_verdicts "${ident[$i]}")" || rc=$?
    case "$rc" in
      0) verdict_record_match "$repo" "$sha" "$((i + 1))" "${who[$i]}" "${ident[$i]}" "$records" ;;
      3) finding L7-VERDICT-ISSUE-MISSING "$repo" "$sha" \
           "verdict-$((i + 1)) cites ${ident[$i]}, which does not exist" ;;
      *) finding L7-VERDICT-UNCHECKED "$repo" "$sha" \
           "the contents of ${ident[$i]} were not checked, so verdict-$((i + 1)) is unverified" ;;
    esac
  done
}

# verdict_record_match <repo> <sha> <index> <reviewer> <issue> <records>
#
# §4.2 rule 3: the reviewer is checked at both ends. Being in {Assay, Warren} on
# the verdict line is L7-VERDICT-WHO; being the agent that actually recorded the
# verdict at the issue is L7-VERDICT-MISATTRIBUTED. Two ends, two codes, because
# a block can be well formed and still cite the wrong author.
verdict_record_match() {
  local repo="$1" sha="$2" idx="$3" want="$4" ident="$5" records="$6"
  local r_who r_disp r_sha matched='' other=''

  while read -r r_who r_disp r_sha _; do
    if [ -z "$r_who" ]; then continue; fi
    if [ "$(lower "$r_disp")" != approve ]; then continue; fi
    if ! sha_eq "$r_sha" "$K_HEAD"; then continue; fi
    if [ "$(lower "$r_who")" = "$(lower "$want")" ]; then matched=1; break; fi
    other="$r_who"
  done <<< "$records"

  if [ -n "$matched" ]; then return 0; fi
  if [ -n "$other" ]; then
    finding L7-VERDICT-MISATTRIBUTED "$repo" "$sha" \
      "$ident records an APPROVE covering $(short "$K_HEAD") by '$other', not by '$want' (verdict-$idx)"
  else
    finding L7-VERDICT-NOT-FOUND "$repo" "$sha" \
      "$ident records no APPROVE by '$want' covering $(short "$K_HEAD") (verdict-$idx)"
  fi
}

commit_touches_crates() {
  local sha="$1" p1
  p1="$(git rev-parse --verify --quiet "$sha^1" 2>/dev/null || true)"
  if [ -n "$p1" ]; then
    [ -n "$(git diff --name-only "$p1" "$sha" -- crates 2>/dev/null)" ]
  else
    [ -n "$(git diff-tree --no-commit-id --name-only -r --root "$sha" -- crates 2>/dev/null)" ]
  fi
}

# ---------------------------------------------------------------------------
# Subcommands
# ---------------------------------------------------------------------------

require_git_repo() {
  git rev-parse --git-dir >/dev/null 2>&1 || die2 "not a git repository: $PWD"
}

report() {
  local repo="$1" sha="$2"
  classify_commit "$repo" "$sha"
  # `OK` and a finding line are mutually exclusive for one commit.
  if [ -z "$EMITTED" ]; then printf 'OK %s %s\n' "$repo" "$sha"; fi
}

cmd_commit() {
  [ "$#" -eq 2 ] || die2 'usage: ci/lane7-attest.sh commit <owner>/<repo> <sha>'
  local repo="$1" sha40
  emit_mode
  require_git_repo
  sha40="$(git rev-parse --verify --quiet "$2^{commit}" || true)"
  [ -n "$sha40" ] || die2 "not a commit in $PWD: $2"
  report "$repo" "$sha40"
}

cmd_repo() {
  [ "$#" -eq 1 ] || die2 'usage: ci/lane7-attest.sh repo <owner>/<repo>'
  emit_mode
  require_git_repo
  load_manifest
  scan_repo "$1"
}

no_floor() {
  # A repository with a protected default branch and no manifest row is not
  # silently skipped: an unmanifested repository and a clean one would otherwise
  # be indistinguishable in this output (ADR-0005 §6.5). The third field is the
  # literal `-` and no RANGE line is emitted.
  EMITTED=''
  finding L7-NO-FLOOR "$1" '-' \
    "no row in $(basename "$FLOOR_MANIFEST"); adding or moving a floor is an ADR-0005 §6.5 amendment"
}

scan_repo() {
  local repo="$1" floor c
  if [ -z "${FLOOR[$repo]+x}" ]; then no_floor "$repo"; return 0; fi
  floor="${FLOOR[$repo]}"

  git rev-parse --verify --quiet "$floor^{commit}" >/dev/null \
    || die2 "$repo: the manifest floor $floor is not a commit in $PWD"
  git rev-parse --verify --quiet 'refs/heads/main^{commit}' >/dev/null \
    || die2 "$repo: refs/heads/main does not resolve in $PWD"

  # First-parent only, exclusive of the floor. A true merge's second-parent
  # subtree is the pull request's own branch history, which never carried an
  # attestation and was never asked to; walking all parents would report every
  # branch commit ever merged.
  local -a commits=()
  while IFS= read -r c; do
    if [ -n "$c" ]; then commits+=("$c"); fi
  done < <(git rev-list --first-parent --reverse "$floor..refs/heads/main")

  printf 'RANGE %s %s\n' "$repo" "${#commits[@]}"
  for c in ${commits[@]+"${commits[@]}"}; do report "$repo" "$c"; done
}

cmd_run() {
  emit_mode
  load_manifest
  local -a scan=()
  if [ "$#" -gt 0 ]; then scan=("$@"); else scan=(${MANIFEST_ORDER[@]+"${MANIFEST_ORDER[@]}"}); fi

  local work repo dir rc
  work="$(mktemp -d)"
  # shellcheck disable=SC2064
  trap "rm -rf '$work'" EXIT

  for repo in ${scan[@]+"${scan[@]}"}; do
    if [ -z "${FLOOR[$repo]+x}" ]; then no_floor "$repo"; continue; fi
    dir="$work/${repo//\//_}"
    # Cloned with no credential — ADR-0005 §8 made these repositories public —
    # so no token reaches a URL that could be echoed into a log. The token is
    # used only for the API lookups.
    note "cloning $repo"
    git clone --quiet "https://github.com/$repo.git" "$dir" 2>/dev/null \
      || die2 "could not clone $repo"
    rc=0
    ( cd "$dir" && scan_repo "$repo" && { [ -z "$EXIT_FINDING" ] || exit 1; } ) || rc=$?
    case "$rc" in
      0) ;;
      1) EXIT_FINDING=1 ;;
      *) exit "$rc" ;;
    esac
  done
}

# ---------------------------------------------------------------------------

main() {
  if [ -n "${LANE7_FIXTURE_DIR:-}" ] && [ ! -d "$LANE7_FIXTURE_DIR" ]; then
    die2 "LANE7_FIXTURE_DIR is set but '$LANE7_FIXTURE_DIR' is not a directory"
  fi

  local sub="${1:-help}"
  if [ "$#" -gt 0 ]; then shift; fi
  case "$sub" in
    commit) cmd_commit "$@" ;;
    repo)   cmd_repo "$@" ;;
    run)    cmd_run "$@" ;;
    help|-h|--help) usage 0 ;;
    *) printf 'lane7-attest: unknown subcommand: %s\n' "$sub" >&2; usage 2 ;;
  esac

  if [ -n "$EXIT_FINDING" ]; then exit 1; fi
  exit 0
}

main "$@"
