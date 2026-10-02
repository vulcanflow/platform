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
command -v jq >/dev/null 2>&1 || { printf 'audit: `jq` is not on PATH\n' >&2; exit 2; }

repos_json="$(gh api "/orgs/$ORG/repos?per_page=100" --paginate 2>&1)" || {
  printf 'audit: cannot list repositories for %s:\n%s\n' "$ORG" "$repos_json" >&2; exit 2; }

# Parsed up front, and its exit status checked, because the alternative is the worst
# failure this script has: if jq fails inside the loop's here-string the loop reads one
# empty line, skips it, and the audit prints "0 gap(s)" having examined nothing.
# `--paginate` concatenates one JSON array per page, so slurp and flatten one level.
repos_tsv="$(printf '%s' "$repos_json" \
  | jq -s -r 'flatten(1) | .[] | [.name, (.private|tostring), (.default_branch // "main")] | @tsv')" || {
  printf 'audit: the repository list for %s did not parse — refusing to report a verdict\n' "$ORG" >&2
  exit 2; }

plan="$(gh api "/orgs/$ORG" --jq '.plan.name' 2>/dev/null || printf unknown)"

gaps=0; watching=0; ok=0

printf 'Repository protection audit — org %s, plan %s, %s\n' \
  "$ORG" "$plan" "$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
printf '%s\n' '----------------------------------------------------------------------------'

# One line per repository: name, visibility, default branch. An organisation with no
# repositories is zero iterations and a clean exit, not an error.
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
  #
  # Classic branch protection is not the only mechanism: a repository can be governed by a
  # *ruleset* instead, which leaves no record at /branches/{branch}/protection. This audit
  # does not evaluate rulesets — a ruleset's rules are not the §8.2 fields and treating them
  # as equivalent is how a weak ruleset gets read as protection. It does ask whether any
  # apply, so the finding says "evaluate this by hand" instead of asserting something false
  # about a branch that may in fact be governed.
  if [ "$protection" != present ]; then
    rules_n=0
    rules="$(gh api "/repos/$ORG/$name/rules/branches/$branch" 2>/dev/null || true)"
    rules_n="$(printf '%s' "$rules" | jq -r 'if type == "array" then length else 0 end' 2>/dev/null || printf 0)"
    printf '  %s %-16s %-7s HOLDS CODE and protection is %s\n' "$(red 'GAP')" "$label" "$vis" "$protection"
    if [ "${rules_n:-0}" -gt 0 ]; then
      printf '                                 %s ruleset rule(s) apply to %s — this audit does not\n' "$rules_n" "$branch"
      printf '                                 evaluate rulesets against ADR-0005 §8.2, so check by hand\n'
      printf '                                 before acting: the branch may already be governed.\n'
    else
      printf '                                 no branch protection and no ruleset: %s is directly\n' "$branch"
      printf '                                 writable, force-pushable and admin-bypassable.\n'
    fi
    gaps=$((gaps + 1)); continue
  fi

  admins="$(printf '%s' "$prot" | jq -r '.enforce_admins.enabled // false')"
  force="$(printf '%s'  "$prot" | jq -r '.allow_force_pushes.enabled // false')"
  dels="$(printf '%s'   "$prot" | jq -r '.allow_deletions.enabled // false')"
  # GitHub reports required checks in two places: the legacy `contexts` array and the newer
  # `checks` array of {context, app_id}. A branch configured through the newer form has an
  # empty `contexts`, so counting only that reads a protected repository as having none.
  contexts="$(printf '%s' "$prot" | jq -r '
    [ (.required_status_checks.contexts // [])[],
      ((.required_status_checks.checks // [])[] | .context) ] | unique | length')"

  defects=()
  [ "$admins" = true ]  || defects+=("enforce_admins is false — an admin can bypass the gate")
  [ "$force"  = false ] || defects+=("allow_force_pushes is true — history on main is rewritable")
  [ "$dels"   = false ] || defects+=("allow_deletions is true — main can be deleted")

  # R2: a repository with a pull-request workflow must have its checks required, not
  # merely reporting. A repository with no workflow has nothing to require — but only a
  # 404 means "no workflow". A 403, a 5xx or a rate-limit answer means we do not know,
  # and "we do not know" is a finding, not an `ok`.
  wf_rc=0
  wf="$(gh api "/repos/$ORG/$name/contents/.github/workflows" 2>&1)" || wf_rc=$?
  if [ $wf_rc -eq 0 ]; then
    [ "$contexts" -eq 0 ] && defects+=("has .github/workflows but 0 required status checks — the checks report, they do not block (ADR-0005 R2)")
  elif printf '%s' "$wf" | grep -qi 'Not Found'; then
    : # no workflow directory, so there is nothing to require
  else
    defects+=("cannot read .github/workflows ($(printf '%s' "$wf" | head -1)) — so whether its checks are required is unknown, and unknown is not ok")
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
done <<< "$repos_tsv"

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
