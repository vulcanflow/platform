#!/usr/bin/env bash
# Local harness for ci/lane-gate.sh. Builds a tiny fixture repo and asserts each
# subcommand's verdict on a set of representative diffs.
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
  mkdir -p ci crates/vf-core/src crates/vf-core/tests fuzz
  cp "$GATE" ci/lane-gate.sh; chmod +x ci/lane-gate.sh
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

# --- 16. Tekton lane-gate task (.tekton/tasks/lane-gate.yaml) bundled with
#     production source: refused ------------------------------------------
setup
mkdir -p .tekton/tasks
printf 'kind: Task\n' > .tekton/tasks/lane-gate.yaml
printf 'pub fn add(a:i32,b:i32)->i32{a+b}\npub fn sub(a:i32,b:i32)->i32{a-b}\n' > crates/vf-core/src/lib.rs
commit tekton-lane-gate-task-plus-prod
check "Tekton lane-gate task (.tekton/tasks/lane-gate.yaml) + prod" partition fail

# Optional: the refusal names the Tekton lane gate, not just ci/lane-gate.sh.
out="$(./ci/lane-gate.sh partition baseline HEAD 2>&1)"; rc=$?
if [ $rc -ne 0 ] && printf '%s' "$out" | grep -q 'Tekton lane gate'; then
  printf 'ok    %-46s %s\n' "refusal text names the Tekton lane gate" "partition"
  pass_count=$((pass_count+1))
else
  printf 'NOT OK %-45s expected refusal text to name the Tekton lane gate\n' "refusal text"
  printf '%s\n' "$out" | sed 's/^/        | /'
  fail_count=$((fail_count+1))
fi

# --- 17. Tekton lane-gate selftest task (.tekton/tasks/lane-gate-selftest.yaml)
#     bundled with production source: refused -----------------------------
setup
mkdir -p .tekton/tasks
printf 'kind: Task\n' > .tekton/tasks/lane-gate-selftest.yaml
printf 'pub fn add(a:i32,b:i32)->i32{a+b}\npub fn sub(a:i32,b:i32)->i32{a-b}\n' > crates/vf-core/src/lib.rs
commit tekton-lane-gate-selftest-plus-prod
check "Tekton lane-gate selftest (.tekton/tasks/lane-gate-selftest.yaml) + prod" partition fail

# --- 18. Tekton lane-gate pipeline (.tekton/pipelines/lane-gate.yaml) bundled
#     with production source: refused --------------------------------------
setup
mkdir -p .tekton/pipelines
printf 'kind: Pipeline\n' > .tekton/pipelines/lane-gate.yaml
printf 'pub fn add(a:i32,b:i32)->i32{a+b}\npub fn sub(a:i32,b:i32)->i32{a-b}\n' > crates/vf-core/src/lib.rs
commit tekton-lane-gate-pipeline-plus-prod
check "Tekton lane-gate pipeline (.tekton/pipelines/lane-gate.yaml) + prod" partition fail

# --- 19. a Tekton lane-gate path bundled with a test file: refused --------
setup
mkdir -p .tekton/tasks
printf 'kind: Task\n' > .tekton/tasks/lane-gate.yaml
printf '#[test]\nfn subs(){assert_eq!(vf_core::sub(3,1),2);}\n' > crates/vf-core/tests/sub.rs
commit tekton-lane-gate-task-plus-test
check "Tekton lane-gate task (.tekton/tasks/lane-gate.yaml) + test" partition fail

# --- 20. Tekton lane-gate task alone: allowed -----------------------------
setup
mkdir -p .tekton/tasks
printf 'kind: Task\n' > .tekton/tasks/lane-gate.yaml
commit tekton-lane-gate-task-only
check "Tekton lane-gate task alone" partition pass

# --- 21. Tekton lane-gate selftest task alone: allowed --------------------
setup
mkdir -p .tekton/tasks
printf 'kind: Task\n' > .tekton/tasks/lane-gate-selftest.yaml
commit tekton-lane-gate-selftest-only
check "Tekton lane-gate selftest alone" partition pass

# --- 22. Tekton lane-gate pipeline alone: allowed -------------------------
setup
mkdir -p .tekton/pipelines
printf 'kind: Pipeline\n' > .tekton/pipelines/lane-gate.yaml
commit tekton-lane-gate-pipeline-only
check "Tekton lane-gate pipeline alone" partition pass

# --- 23. all three Tekton lane-gate paths together with NEUTRAL files:
#     allowed ---------------------------------------------------------------
setup
mkdir -p .tekton/tasks .tekton/pipelines
printf 'kind: Task\n' > .tekton/tasks/lane-gate.yaml
printf 'kind: Task\n' > .tekton/tasks/lane-gate-selftest.yaml
printf 'kind: Pipeline\n' > .tekton/pipelines/lane-gate.yaml
printf '# docs\nneutral tweak\n' > README.md
printf '# tekton\n' > .tekton/README.md
commit tekton-lane-gate-all-three-plus-neutral
check "all three Tekton lane-gate paths + NEUTRAL files" partition pass

# --- 24. Tekton rust-check task (NEUTRAL) alongside production source:
#     allowed (control: a non-gate .tekton/ path stays NEUTRAL) -------------
setup
mkdir -p .tekton/tasks
printf 'kind: Task\n' > .tekton/tasks/rust-check.yaml
printf 'pub fn add(a:i32,b:i32)->i32{a+b}\npub fn sub(a:i32,b:i32)->i32{a-b}\n' > crates/vf-core/src/lib.rs
commit tekton-rust-check-plus-prod
check "Tekton rust-check (NEUTRAL) + prod" partition pass

# --- 25. .tekton/kustomization.yaml bundled with production source: refused -
setup
mkdir -p .tekton
printf 'kind: Kustomization\n' > .tekton/kustomization.yaml
printf 'pub fn add(a:i32,b:i32)->i32{a+b}\npub fn sub(a:i32,b:i32)->i32{a-b}\n' > crates/vf-core/src/lib.rs
commit kustomization-plus-prod
check ".tekton/kustomization.yaml + prod" partition fail

# --- 26. .tekton/kustomization.yaml alone: allowed ------------------------
setup
mkdir -p .tekton
printf 'kind: Kustomization\n' > .tekton/kustomization.yaml
commit kustomization-only
check ".tekton/kustomization.yaml alone" partition pass

# --- 27. .tekton/kustomization.yaml + .tekton/README.md (NEUTRAL): allowed -
setup
mkdir -p .tekton
printf 'kind: Kustomization\n' > .tekton/kustomization.yaml
printf '# tekton\n' > .tekton/README.md
commit kustomization-plus-tekton-readme
check ".tekton/kustomization.yaml + .tekton/README.md" partition pass

# --- 28. .tekton/triggers/kustomization.yaml (NEUTRAL) + prod: allowed
#     (control: the trigger-slice manifest is not swept into GATE) ---------
setup
mkdir -p .tekton/triggers
printf 'kind: Kustomization\n' > .tekton/triggers/kustomization.yaml
printf 'pub fn add(a:i32,b:i32)->i32{a+b}\npub fn sub(a:i32,b:i32)->i32{a-b}\n' > crates/vf-core/src/lib.rs
commit tekton-triggers-kustomization-plus-prod
check ".tekton/triggers/kustomization.yaml (NEUTRAL) + prod" partition pass

# --- 29. .tekton/pipelineruns/lane-gate.yaml (NEUTRAL) + prod: allowed
#     (control: GATE classification is exact-path, not a lane-gate.yaml
#     basename glob under .tekton/**) --------------------------------------
setup
mkdir -p .tekton/pipelineruns
printf 'kind: PipelineRun\n' > .tekton/pipelineruns/lane-gate.yaml
printf 'pub fn add(a:i32,b:i32)->i32{a+b}\npub fn sub(a:i32,b:i32)->i32{a-b}\n' > crates/vf-core/src/lib.rs
commit tekton-pipelineruns-lane-gate-plus-prod
check "Tekton pipelineruns/lane-gate.yaml (NEUTRAL) + prod" partition pass

# --- 30. .tekton/kustomization.yml (VFL-799 alias) bundled with production
#     source: refused -------------------------------------------------------
setup
mkdir -p .tekton
printf 'kind: Kustomization\n' > .tekton/kustomization.yml
printf 'pub fn add(a:i32,b:i32)->i32{a+b}\npub fn sub(a:i32,b:i32)->i32{a-b}\n' > crates/vf-core/src/lib.rs
commit kustomization-yml-plus-prod
check ".tekton/kustomization.yml + prod" partition fail

# --- 31. .tekton/Kustomization (VFL-799 alias) bundled with production
#     source: refused -------------------------------------------------------
setup
mkdir -p .tekton
printf 'kind: Kustomization\n' > .tekton/Kustomization
printf 'pub fn add(a:i32,b:i32)->i32{a+b}\npub fn sub(a:i32,b:i32)->i32{a-b}\n' > crates/vf-core/src/lib.rs
commit kustomization-capital-plus-prod
check ".tekton/Kustomization + prod" partition fail

# --- 32. renaming .tekton/kustomization.yaml to the .yml alias, bundled with
#     production source: refused (changed_files lists a rename under both its
#     old and its new path, so classify_path sees .tekton/kustomization.yaml
#     and .tekton/kustomization.yml; VFL-813 made the old path visible too) -
setup
mkdir -p .tekton
printf 'kind: Kustomization\n' > .tekton/kustomization.yaml
git add -A >/dev/null; git commit -qm add-kustomization-yaml
git branch -f baseline
git mv .tekton/kustomization.yaml .tekton/kustomization.yml
printf 'pub fn add(a:i32,b:i32)->i32{a+b}\npub fn sub(a:i32,b:i32)->i32{a-b}\n' > crates/vf-core/src/lib.rs
commit rename-kustomization-yaml-to-yml-plus-prod
check "git mv .tekton/kustomization.yaml -> .yml + prod" partition fail

# --- 33. .tekton/triggers/kustomization.yml (NEUTRAL) + prod: allowed
#     (control: the trigger-slice manifest stays NEUTRAL even under the .yml
#     alias name; LOW-2 scope is unchanged by VFL-799) ---------------------
setup
mkdir -p .tekton/triggers
printf 'kind: Kustomization\n' > .tekton/triggers/kustomization.yml
printf 'pub fn add(a:i32,b:i32)->i32{a+b}\npub fn sub(a:i32,b:i32)->i32{a-b}\n' > crates/vf-core/src/lib.rs
commit tekton-triggers-kustomization-yml-plus-prod
check ".tekton/triggers/kustomization.yml (NEUTRAL) + prod" partition pass

# --- 34. .tekton/kustomization.yaml bundled with a TEST file: refused
#     (VFL-797 LOW-8: GATE + TEST must refuse, not just GATE + PROD) -------
setup
mkdir -p .tekton
printf 'kind: Kustomization\n' > .tekton/kustomization.yaml
printf '#[test]\nfn subs(){assert_eq!(vf_core::sub(3,1),2);}\n' > crates/vf-core/tests/sub.rs
commit kustomization-yaml-plus-test
check ".tekton/kustomization.yaml + test" partition fail

# --- 35. .tekton/kustomization.yml alone: allowed -------------------------
setup
mkdir -p .tekton
printf 'kind: Kustomization\n' > .tekton/kustomization.yml
commit kustomization-yml-only
check ".tekton/kustomization.yml alone" partition pass

# --- 36. .tekton/Kustomization alone: allowed -----------------------------
setup
mkdir -p .tekton
printf 'kind: Kustomization\n' > .tekton/Kustomization
commit kustomization-capital-only
check ".tekton/Kustomization alone" partition pass

# --- 37. .tekton/kustomization.yml + .tekton/README.md (NEUTRAL): allowed -
setup
mkdir -p .tekton
printf 'kind: Kustomization\n' > .tekton/kustomization.yml
printf '# tekton\n' > .tekton/README.md
commit kustomization-yml-plus-tekton-readme
check ".tekton/kustomization.yml + .tekton/README.md" partition pass

# --- 38. ci/lane-gate.sh deleted, bundled with production source: refused
#     (VFL-813: a deleted path keeps its class, so the gate's own script
#     deleted next to prod is GATE + PROD). Committing the deletion removes
#     ./ci/lane-gate.sh from the fixture's working tree, and check() execs
#     that relative path, so the real gate script is copied back onto disk
#     (untracked) right after the commit. baseline...HEAD still records the
#     deletion since that diff reads commits, not the working tree, so the
#     verdict still comes from the gate under test and not from a missing
#     executable, which also exits non-zero and would look like a pass. ----
setup
rm ci/lane-gate.sh
printf 'pub fn add(a:i32,b:i32)->i32{a+b}\npub fn sub(a:i32,b:i32)->i32{a-b}\n' > crates/vf-core/src/lib.rs
commit delete-lane-gate-sh-plus-prod
cp "$GATE" ci/lane-gate.sh; chmod +x ci/lane-gate.sh
check "delete ci/lane-gate.sh + prod" partition fail

# --- 39. .tekton/kustomization.yaml deleted, bundled with production source:
#     refused (VFL-813: a deleted GATE path counts as GATE; the base must
#     carry the file for the deletion to show up as a D row) --------------
setup
mkdir -p .tekton
printf 'kind: Kustomization\n' > .tekton/kustomization.yaml
git add -A >/dev/null; git commit -qm add-kustomization-yaml
git branch -f baseline
git rm -q .tekton/kustomization.yaml
printf 'pub fn add(a:i32,b:i32)->i32{a+b}\npub fn sub(a:i32,b:i32)->i32{a-b}\n' > crates/vf-core/src/lib.rs
commit delete-kustomization-yaml-plus-prod
check "delete .tekton/kustomization.yaml + prod" partition fail

# --- 40. renaming .tekton/tasks/lane-gate.yaml to a NEUTRAL name, bundled
#     with production source: refused (VFL-813: changed_files lists an R row
#     under its old path too, so classify_path sees the GATE source path
#     even though the destination is NEUTRAL; the base must carry the file) -
setup
mkdir -p .tekton/tasks
printf 'kind: Task\n' > .tekton/tasks/lane-gate.yaml
git add -A >/dev/null; git commit -qm add-lane-gate-task
git branch -f baseline
git mv .tekton/tasks/lane-gate.yaml .tekton/tasks/lane-gate.yaml.disabled
printf 'pub fn add(a:i32,b:i32)->i32{a+b}\npub fn sub(a:i32,b:i32)->i32{a-b}\n' > crates/vf-core/src/lib.rs
commit rename-lane-gate-task-to-disabled-plus-prod
check "git mv .tekton/tasks/lane-gate.yaml -> .disabled + prod" partition fail

# --- 41. a TEST file deleted, bundled with production source: refused
#     (VFL-813: a deleted TEST path still counts as TEST, so this is a lane
#     crossing the same as editing the test file would have been) ---------
setup
rm crates/vf-core/tests/more.rs
printf 'pub fn add(a:i32,b:i32)->i32{a+b}\npub fn sub(a:i32,b:i32)->i32{a-b}\n' > crates/vf-core/src/lib.rs
commit delete-test-file-plus-prod
check "delete TEST file (crates/vf-core/tests/more.rs) + prod" partition fail

# --- 42. a NEUTRAL file deleted, bundled with production source: allowed
#     (control: a deleted NEUTRAL path still carries no lane, same as before
#     VFL-813) ----------------------------------------------------------
setup
rm README.md
printf 'pub fn add(a:i32,b:i32)->i32{a+b}\npub fn sub(a:i32,b:i32)->i32{a-b}\n' > crates/vf-core/src/lib.rs
commit delete-readme-plus-prod
check "delete NEUTRAL file (README.md) + prod" partition pass

# --- 43. a GATE file deleted alone: allowed (VFL-813: a deletion with no
#     accompanying PROD or TEST passes, same as any other GATE-only change;
#     the base must carry the file for the deletion to show up as a D row) -
setup
mkdir -p .tekton/tasks
printf 'kind: Task\n' > .tekton/tasks/lane-gate.yaml
git add -A >/dev/null; git commit -qm add-lane-gate-task
git branch -f baseline
git rm -q .tekton/tasks/lane-gate.yaml
commit delete-lane-gate-task-alone
check "delete GATE file alone (.tekton/tasks/lane-gate.yaml)" partition pass

printf '\n%s passed, %s failed\n' "$pass_count" "$fail_count"
[ "$fail_count" -eq 0 ]
