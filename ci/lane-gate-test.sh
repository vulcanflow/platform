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

printf '\n%s passed, %s failed\n' "$pass_count" "$fail_count"
[ "$fail_count" -eq 0 ]
