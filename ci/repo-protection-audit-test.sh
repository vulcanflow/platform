#!/usr/bin/env bash
# Local harness for ci/repo-protection-audit.sh.
#
# The audit reads the live GitHub API, so the only honest way to show it refusing something
# is to feed it an organisation that has the defect. This harness does that with a stub `gh`
# on PATH, serving fixture responses, and asserts the audit's verdict and exit code for each
# organisation shape — including the ones we hope never to see for real.
#
# Usage: ci/repo-protection-audit-test.sh ci/repo-protection-audit.sh
set -uo pipefail

unset GIT_CONFIG_GLOBAL GIT_CONFIG_SYSTEM GIT_CONFIG_COUNT
export PATH="/usr/bin:/bin:/usr/local/bin"

AUDIT="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
W="$(mktemp -d)"
trap 'rm -rf "$W"' EXIT
mkdir -p "$W/bin" "$W/fix"

# ---------------------------------------------------------------------------
# The stub `gh`. Serves $FIX/<path with / and ? flattened>; a leading `RC=<n>`
# line in a fixture makes the call fail with that exit code, which is how the
# real 403s and 409s are reproduced.
# ---------------------------------------------------------------------------
#
# The audit is invoked through `env -i` so that nothing from the surrounding shell can
# reach it. Without that, the Paperclip GitHub launcher's GH_CONFIG_DIR and token vars
# leak in and the audit talks to the real organisation — which looks like a pass for the
# wrong reason, and is slow enough to look like a hang.
# ---------------------------------------------------------------------------
cat > "$W/bin/gh" <<'STUB'
#!/usr/bin/env bash
# stub gh — only `gh api <path> [--jq filter] [--paginate]` is supported.
[ "${1:-}" = api ] || { echo "stub gh: unsupported: $*" >&2; exit 90; }
path="$2"; shift 2
filter=""
while [ $# -gt 0 ]; do
  case "$1" in
    --jq) filter="$2"; shift 2 ;;
    --paginate|-q) shift ;;
    *) shift ;;
  esac
done
key="$(printf '%s' "${path#/}" | tr '/?=&' '____')"
f="$FIX/$key"
if [ ! -f "$f" ]; then printf 'stub gh: no fixture for %s (looked for %s)\n' "$path" "$key" >&2; exit 91; fi
rc=0
if head -1 "$f" | grep -q '^RC='; then rc="$(head -1 "$f" | cut -d= -f2)"; body="$(tail -n +2 "$f")";
else body="$(cat "$f")"; fi
if [ "$rc" -ne 0 ]; then printf '%s\n' "$body" >&2; exit "$rc"; fi
if [ -n "$filter" ]; then printf '%s' "$body" | jq -r "$filter"; else printf '%s\n' "$body"; fi
STUB
chmod +x "$W/bin/gh"
export PATH="$W/bin:$PATH"

pass_count=0; fail_count=0
FIXDIR="$W/fix"; export FIX="$FIXDIR"

reset() { rm -rf "$FIXDIR"; mkdir -p "$FIXDIR"; printf '{"plan":{"name":"free"}}' > "$FIXDIR/orgs_vulcanflow"; }

# repos <json array>
repos() { printf '%s' "$1" > "$FIXDIR/orgs_vulcanflow_repos_per_page_100"; }

repo_entry() { printf '{"name":"%s","private":%s,"default_branch":"main"}' "$1" "$2"; }

empty_repo()  { printf 'RC=1\n{"message":"Git Repository is empty.","status":"409"}' > "$FIXDIR/repos_vulcanflow_${1}_commits_per_page_1"; }
coded_repo()  { printf '[{"sha":"abc1234"}]' > "$FIXDIR/repos_vulcanflow_${1}_commits_per_page_1"; }

prot_refused() { printf 'RC=1\n{"message":"Upgrade to GitHub Pro or make this repository public to enable this feature.","status":"403"}' > "$FIXDIR/repos_vulcanflow_${1}_branches_main_protection"; }
prot_absent()  { printf 'RC=1\n{"message":"Branch not protected","status":"404"}' > "$FIXDIR/repos_vulcanflow_${1}_branches_main_protection"; }
# prot_good <repo> <admins> <force> <deletions> <context count>
prot_good() {
  local ctx="[]"; [ "${5:-0}" -gt 0 ] && ctx="$(seq 1 "$5" | sed 's/.*/"check&"/' | paste -sd, -)" && ctx="[$ctx]"
  printf '{"required_status_checks":{"strict":true,"contexts":%s},"enforce_admins":{"enabled":%s},"allow_force_pushes":{"enabled":%s},"allow_deletions":{"enabled":%s}}' \
    "$ctx" "$2" "$3" "$4" > "$FIXDIR/repos_vulcanflow_${1}_branches_main_protection"
}
has_workflows() { printf '[{"name":"lane-gate.yml","type":"file"}]' > "$FIXDIR/repos_vulcanflow_${1}_contents_.github_workflows"; }
no_workflows()  { printf 'RC=1\n{"message":"Not Found","status":"404"}' > "$FIXDIR/repos_vulcanflow_${1}_contents_.github_workflows"; }

# check <name> <expected exit> <expected substring in output>
check() {
  local name="$1" expect_rc="$2" expect_txt="$3" out rc=0
  out="$(env -i PATH="$W/bin:/usr/bin:/bin" FIX="$FIXDIR" HOME="$W" \
          bash "$AUDIT" vulcanflow 2>&1)" || rc=$?
  local plain; plain="$(printf '%s' "$out" | sed 's/\x1b\[[0-9;]*m//g')"
  if [ "$rc" = "$expect_rc" ] && printf '%s' "$plain" | grep -q "$expect_txt"; then
    printf 'ok    %-52s exit=%s\n' "$name" "$rc"; pass_count=$((pass_count+1))
  else
    printf 'NOT OK %-51s expected exit=%s and /%s/, got exit=%s\n' "$name" "$expect_rc" "$expect_txt" "$rc"
    printf '%s\n' "$plain" | sed 's/^/        | /'
    fail_count=$((fail_count+1))
  fi
}

# --- 1. today's real shape: empty private repos are watched, not failed ------
reset
repos "[$(repo_entry platform false),$(repo_entry vf-authz true)]"
coded_repo platform; prot_good platform true false false 4; has_workflows platform
empty_repo vf-authz; prot_refused vf-authz
check "empty private repo is watched, not a gap" 0 "1 watched"
check "protected repo with required checks passes" 0 "0 gap"

# --- 2. the event the deferral is waiting for: first push into a private repo -
reset
repos "[$(repo_entry vf-authz true)]"
coded_repo vf-authz; prot_refused vf-authz
check "code pushed into unprotectable private repo" 1 "GAP"
check "   and it names the writable main" 1 "directly writable and force-pushable"

# --- 3. a repo with code and no protection at all ---------------------------
reset
repos "[$(repo_entry infra false)]"
coded_repo infra; prot_absent infra
check "code in a public repo with no protection" 1 "GAP"

# --- 4. protection present but an admin can bypass it ----------------------
reset
repos "[$(repo_entry platform false)]"
coded_repo platform; prot_good platform false false false 4; has_workflows platform
check "enforce_admins false is a gap" 1 "enforce_admins is false"

# --- 5. protection present but main is force-pushable ---------------------
reset
repos "[$(repo_entry platform false)]"
coded_repo platform; prot_good platform true true false 4; has_workflows platform
check "allow_force_pushes true is a gap" 1 "allow_force_pushes is true"

# --- 6. protection present but main is deletable -------------------------
reset
repos "[$(repo_entry platform false)]"
coded_repo platform; prot_good platform true false true 4; has_workflows platform
check "allow_deletions true is a gap" 1 "allow_deletions is true"

# --- 7. ADR-0005 R2: workflows that report but do not block --------------
reset
repos "[$(repo_entry platform false)]"
coded_repo platform; prot_good platform true false false 0; has_workflows platform
check "workflows present, 0 required checks (R2)" 1 "they do not block"

# --- 8. no workflow yet, so nothing to require --------------------------
reset
repos "[$(repo_entry docs false)]"
coded_repo docs; prot_good docs true false false 0; no_workflows docs
check "no workflow, 0 required checks is fine" 0 "0 gap"

# --- 9. an empty repo that is protected anyway --------------------------
reset
repos "[$(repo_entry vf-api false)]"
empty_repo vf-api; prot_good vf-api true false false 0; no_workflows vf-api
check "empty but protected reads as ok" 0 "empty, and protected anyway"

# --- 10. the audit cannot complete: that is not a pass -----------------
reset
rm -f "$FIXDIR/orgs_vulcanflow_repos_per_page_100"
check "unreadable org exits 2, never 0" 2 "cannot list repositories"

# --- 11. a repo whose commits cannot be read is a gap, not a pass ------
reset
repos "[$(repo_entry vf-web true)]"
printf 'RC=1\n{"message":"Bad credentials","status":"401"}' > "$FIXDIR/repos_vulcanflow_vf-web_commits_per_page_1"
check "unreadable commits counts as a gap" 1 "cannot read commits"

printf '\n%s passed, %s failed\n' "$pass_count" "$fail_count"
[ "$fail_count" -eq 0 ]
