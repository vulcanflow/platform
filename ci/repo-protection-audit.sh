#!/usr/bin/env bash
# repo-protection-audit.sh — the detection gate for the deferred branch-protection gap.
#
# Why this exists
# ---------------
# ADR-0005 §8 records that `vulcanflow` is on the GitHub Free plan, which refuses branch
# protection on a private repository. Four repositories were made public to get protection;
# the other eleven stayed private and therefore unprotectable. The board decided on
# 2026-10-02 to defer the upgrade rather than pay for it, on the grounds that all eleven are
# empty and an empty repository has nothing to protect.
#
# That reasoning holds exactly until the first push. A deferral whose safety depends on
# somebody remembering to look is not a control, so this script is the looking. It answers
# one question about the whole organisation:
#
#     is there a repository that holds code and is not protected?
#
# and exits non-zero if there is. It is also the control for ADR-0005 R2: a repository that
# gains its first pull-request workflow must have those checks wired as *required*, and a
# workflow that merely reports is caught here as a gap.
#
# This runs against the GitHub API, not against a diff, so it cannot run as a pull-request
# check in GitHub Actions — the per-repository GITHUB_TOKEN cannot read the organisation's
# other repositories. It is driven by the Paperclip routine "Org repo protection audit",
# which runs it on a schedule under the brokered credential, and it can be run by hand at
# any time with an authenticated `gh`.
#
# Usage: ci/repo-protection-audit.sh [org]
#
# Exit codes
#   0  no repository holds code without protection
#   1  at least one GAP — a repository holds code and is not protected as ADR-0005 §8.2 requires
#   2  the audit could not be completed (no `gh`, not authenticated, org unreadable)

set -uo pipefail

ORG="${1:-vulcanflow}"

red()   { printf '\033[31m%s\033[0m' "$1"; }
green() { printf '\033[32m%s\033[0m' "$1"; }
amber() { printf '\033[33m%s\033[0m' "$1"; }

command -v gh >/dev/null 2>&1 || { printf 'audit: `gh` is not on PATH\n' >&2; exit 2; }

repos_json="$(gh api "/orgs/$ORG/repos?per_page=100" --paginate 2>&1)" || {
  printf 'audit: cannot list repositories for %s:\n%s\n' "$ORG" "$repos_json" >&2; exit 2; }

plan="$(gh api "/orgs/$ORG" --jq '.plan.name' 2>/dev/null || printf unknown)"

gaps=0; watching=0; ok=0

printf 'Repository protection audit — org %s, plan %s, %s\n' \
  "$ORG" "$plan" "$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
printf '%s\n' '----------------------------------------------------------------------------'

# `jq -r` over the list keeps one line per repository: name, visibility, default branch.
while IFS=$'\t' read -r name private branch; do
  [ -n "$name" ] || continue
  branch="${branch:-main}"
  label="$name"

  # ---- does it hold code? ------------------------------------------------------------
  # An empty repository answers 409 "Git Repository is empty" on /commits. That 409 is
  # the whole deferral: it is the thing that stops being true at the first push.
  commits_rc=0
  commits="$(gh api "/repos/$ORG/$name/commits?per_page=1" 2>&1)" || commits_rc=$?
  if [ $commits_rc -ne 0 ] && printf '%s' "$commits" | grep -q 'Repository is empty'; then
    has_code=no
  elif [ $commits_rc -ne 0 ]; then
    printf '  %s  %-16s cannot read commits: %s\n' "$(red '??')" "$label" \
      "$(printf '%s' "$commits" | head -1)"
    gaps=$((gaps + 1)); continue
  else
    has_code=yes
  fi

  # ---- is the default branch protected? ----------------------------------------------
  prot_rc=0
  prot="$(gh api "/repos/$ORG/$name/branches/$branch/protection" 2>&1)" || prot_rc=$?
  if [ $prot_rc -eq 0 ]; then
    protection=present
  elif printf '%s' "$prot" | grep -q 'Upgrade to GitHub'; then
    protection=refused-by-plan
  elif printf '%s' "$prot" | grep -qi 'Branch not protected\|Not Found'; then
    protection=absent
  else
    protection="unreadable: $(printf '%s' "$prot" | head -1)"
  fi

  vis=public; [ "$private" = "true" ] && vis=private

  # ---- empty: nothing to protect yet, but say so out loud ---------------------------
  if [ "$has_code" = no ]; then
    if [ "$protection" = present ]; then
      printf '  %s  %-16s %-7s empty, and protected anyway\n' "$(green 'ok')" "$label" "$vis"
      ok=$((ok + 1))
    else
      printf '  %s  %-16s %-7s empty — protection %s. Deferred per ADR-0005 §8.5; this\n' \
        "$(amber 'watch')" "$label" "$vis" "$protection"
      printf '                                 line turns into a GAP on its first push.\n'
      watching=$((watching + 1))
    fi
    continue
  fi

  # ---- holds code: protection is not optional ---------------------------------------
  if [ "$protection" != present ]; then
    printf '  %s %-16s %-7s HOLDS CODE and protection is %s\n' "$(red 'GAP')" "$label" "$vis" "$protection"
    printf '                                 main is directly writable and force-pushable.\n'
    gaps=$((gaps + 1)); continue
  fi

  admins="$(printf '%s' "$prot" | jq -r '.enforce_admins.enabled // false')"
  force="$(printf '%s'  "$prot" | jq -r '.allow_force_pushes.enabled // false')"
  dels="$(printf '%s'   "$prot" | jq -r '.allow_deletions.enabled // false')"
  contexts="$(printf '%s' "$prot" | jq -r '[.required_status_checks.contexts // []] | flatten | length')"

  defects=()
  [ "$admins" = true ]  || defects+=("enforce_admins is false — an admin can bypass the gate")
  [ "$force"  = false ] || defects+=("allow_force_pushes is true — history on main is rewritable")
  [ "$dels"   = false ] || defects+=("allow_deletions is true — main can be deleted")

  # R2: a repository with a pull-request workflow must have its checks required, not
  # merely reporting. A repository with no workflow has nothing to require.
  wf_rc=0
  wf="$(gh api "/repos/$ORG/$name/contents/.github/workflows" 2>&1)" || wf_rc=$?
  if [ $wf_rc -eq 0 ] && [ "$contexts" -eq 0 ]; then
    defects+=("has .github/workflows but 0 required status checks — the checks report, they do not block (ADR-0005 R2)")
  fi

  if [ ${#defects[@]} -gt 0 ]; then
    printf '  %s %-16s %-7s protected, with defects:\n' "$(red 'GAP')" "$label" "$vis"
    printf '                                 - %s\n' "${defects[@]}"
    gaps=$((gaps + 1))
  else
    printf '  %s  %-16s %-7s protected, %s required check(s), admins included\n' \
      "$(green 'ok')" "$label" "$vis" "$contexts"
    ok=$((ok + 1))
  fi
done <<< "$(printf '%s' "$repos_json" | jq -r '.[] | [.name, (.private|tostring), .default_branch] | @tsv')"

printf '%s\n' '----------------------------------------------------------------------------'
printf '%s ok, %s watched (empty, unprotectable), %s gap(s)\n' "$ok" "$watching" "$gaps"

if [ "$gaps" -gt 0 ]; then
  cat >&2 <<'EOF'

A repository holds code without the protection ADR-0005 §8.2 requires.

On the Free plan a private repository cannot be protected at all, so for a private
repository there are exactly two ways to close this and neither is "later":

  - upgrade the org to GitHub Team (ADR-0005 §8 option A, about $4/month at one
    filled seat), then PUT the §8.2 settings on the repository; or
  - make the repository public, which requires the §8.1 pre-publication check first.

Until one of those happens, main in that repository is directly writable, force-pushable
and bypassable by an admin, and the delivery lanes are advisory there. Raise it on the
decisions desk; do not merge service code into an unprotected repository.
EOF
  exit 1
fi

printf 'No repository holds code without protection.\n'
exit 0
