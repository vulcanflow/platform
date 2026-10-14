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
  # Pinned so cases 46-47 (non-ASCII filenames) see the C-quoting bug/fix
  # regardless of the runner's global gitconfig.
  git config core.quotePath true
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

# check_contains <name> <subcommand> <expected: pass|fail> <substring>...
# Like check(), but also asserts the subcommand's combined stdout+stderr output
# contains every given literal substring (grep -F, so a path's dots are not
# treated as regex wildcards). Needed for cases whose refusal text must name
# specific paths, not just land on the right pass/fail verdict.
check_contains() {
  local name="$1" sub="$2" expect="$3"; shift 3
  local out rc got=pass
  out="$(./ci/lane-gate.sh "$sub" baseline HEAD 2>&1)"; rc=$?
  [ $rc -ne 0 ] && got=fail
  if [ "$got" != "$expect" ]; then
    printf 'NOT OK %-45s %s: expected %s got %s\n' "$name" "$sub" "$expect" "$got"
    printf '%s\n' "$out" | sed 's/^/        | /'
    fail_count=$((fail_count+1))
    return
  fi
  if [ "$#" -eq 0 ]; then
    printf 'NOT OK %-45s %s: check_contains called with no substrings to verify\n' "$name" "$sub"
    fail_count=$((fail_count+1))
    return
  fi
  local missing=() s
  for s in "$@"; do
    if [ -z "$s" ] || ! grep -qF -- "$s" <<<"$out"; then
      missing+=("$s")
    fi
  done
  if [ ${#missing[@]} -eq 0 ]; then
    printf 'ok    %-46s %s=%s (output verified)\n' "$name" "$sub" "$got"
    pass_count=$((pass_count+1))
  else
    printf 'NOT OK %-45s %s: output missing: %s\n' "$name" "$sub" "${missing[*]}"
    printf '%s\n' "$out" | sed 's/^/        | /'
    fail_count=$((fail_count+1))
  fi
}

# install_git_stub <printf-literal>
# Writes an executable git stub under $W/stub, ahead of the real git on PATH,
# that ignores every argument and prints the literal record given. Written
# under $W, not inside the fixture repo, so the fixture's own git commands
# never see it on disk.
install_git_stub() {
  mkdir -p "$W/stub"
  cat > "$W/stub/git" <<EOF
#!/usr/bin/env bash
printf '$1'
EOF
  chmod +x "$W/stub/git"
}

# check_contains_stubbed_git <name> <expected: pass|fail> <substring>...
# Like check_contains(), but runs partition with the git stub from
# install_git_stub() ahead of PATH, so changed_files() parses the stub's
# record instead of a real diff. partition calls git exactly once, so this
# is enough to reach every branch of changed_files' parser. BASH_ENV is
# cleared for this call: the agent runners export BASH_ENV pointing at a
# .bashrc that resets PATH, and bash sources it when ./ci/lane-gate.sh
# starts, which would otherwise drop the stub before partition ever calls
# git.
check_contains_stubbed_git() {
  local name="$1" expect="$2"; shift 2
  local out rc got=pass
  out="$(BASH_ENV= PATH="$W/stub:$PATH" ./ci/lane-gate.sh partition baseline HEAD 2>&1)"; rc=$?
  [ $rc -ne 0 ] && got=fail
  if [ "$got" != "$expect" ]; then
    printf 'NOT OK %-45s %s: expected %s got %s\n' "$name" partition "$expect" "$got"
    printf '%s\n' "$out" | sed 's/^/        | /'
    fail_count=$((fail_count+1))
    return
  fi
  if [ "$#" -eq 0 ]; then
    printf 'NOT OK %-45s %s: check_contains called with no substrings to verify\n' "$name" partition
    fail_count=$((fail_count+1))
    return
  fi
  local missing=() s
  for s in "$@"; do
    if [ -z "$s" ] || ! grep -qF -- "$s" <<<"$out"; then
      missing+=("$s")
    fi
  done
  if [ ${#missing[@]} -eq 0 ]; then
    printf 'ok    %-46s %s=%s (output verified)\n' "$name" partition "$got"
    pass_count=$((pass_count+1))
  else
    printf 'NOT OK %-45s %s: output missing: %s\n' "$name" partition "${missing[*]}"
    printf '%s\n' "$out" | sed 's/^/        | /'
    fail_count=$((fail_count+1))
  fi
}

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

# --- 44. GATE + PROD: the gate refusal names the GATE path, not just a note
#     count (VFL-814 / VFL-787 LOW-6: before this, the gate refusal printed
#     only "production files: N   test files: N   gate files: N", so with
#     several GATE paths an author could not tell which file was classed
#     GATE) ----------------------------------------------------------------
setup
mkdir -p .tekton/tasks
printf 'kind: Task\n' > .tekton/tasks/lane-gate.yaml
printf 'pub fn add(a:i32,b:i32)->i32{a+b}\npub fn sub(a:i32,b:i32)->i32{a-b}\n' > crates/vf-core/src/lib.rs
commit tekton-lane-gate-task-plus-prod-names-paths
check_contains "GATE + PROD refusal names the GATE and PROD paths" partition fail \
  'gate files in this diff:' '.tekton/tasks/lane-gate.yaml' \
  'production source in this diff:' 'crates/vf-core/src/lib.rs'

# --- 45. GATE + TEST: the gate refusal names both the GATE path and the
#     TEST path (VFL-814 / VFL-787 LOW-6) -----------------------------------
setup
mkdir -p .tekton/tasks
printf 'kind: Task\n' > .tekton/tasks/lane-gate.yaml
printf '#[test]\nfn two(){assert!(true);}\n#[test]\nfn three(){assert!(true);}\n#[test]\nfn four(){assert!(true);}\n' > crates/vf-core/tests/more.rs
commit tekton-lane-gate-task-plus-test-names-paths
check_contains "GATE + TEST refusal names the GATE and TEST paths" partition fail \
  'gate files in this diff:' '.tekton/tasks/lane-gate.yaml' \
  'test files in this diff:' 'crates/vf-core/tests/more.rs'

# --- 46. a TEST file with a non-ASCII name, bundled with production source:
#     refused (VFL-836 / VFL-819 LOW-1/LOW-2: with core.quotePath=true, git
#     name-status C-quotes café.rs to "caf\303\251.rs"; its leading '"' made
#     classify_path call it NEUTRAL, so this used to pass) -----------------
setup
printf '#[test]\nfn cafe(){assert!(true);}\n' > "crates/vf-core/tests/café.rs"
printf 'pub fn add(a:i32,b:i32)->i32{a+b}\npub fn sub(a:i32,b:i32)->i32{a-b}\n' > crates/vf-core/src/lib.rs
commit test-non-ascii-name-plus-prod
check_contains "TEST file with a non-ASCII name (café.rs) + prod" partition fail \
  'test files: 1'

# --- 47. a PROD file with a non-ASCII name, bundled with a test edit:
#     refused (same quoting bug on the PROD side) --------------------------
setup
printf 'pub fn mul(a:i32,b:i32)->i32{a*b}\n' > "crates/vf-core/src/café.rs"
printf '#[test]\nfn adds(){assert_eq!(vf_core::add(1,2),3);}\n#[test]\nfn more(){assert!(true);}\n' > crates/vf-core/tests/add.rs
commit prod-non-ascii-name-plus-test
check_contains "PROD file with a non-ASCII name (café.rs) + test edit" partition fail \
  'production files: 1'

# --- 48. a TEST file whose name has a quote, bundled with production source:
#     refused (core.quotePath=false alone would not help here: a '"', '\',
#     tab or newline is quoted unconditionally, so this name is NEUTRAL on
#     the parent gate regardless of that setting) --------------------------
setup
f='crates/vf-core/tests/has"quote.rs'
printf '#[test]\nfn t(){assert!(true);}\n' > "$f"
printf 'pub fn add(a:i32,b:i32)->i32{a+b}\npub fn sub(a:i32,b:i32)->i32{a-b}\n' > crates/vf-core/src/lib.rs
commit test-quote-name-plus-prod
check_contains "TEST file with a quote in its name + prod" partition fail \
  'test files: 1'

# --- 49. a PROD file under a directory whose name holds a newline, bundled
#     with a test edit: refused (a split on that newline would read
#     crates/a and b/src/lib.rs, both NEUTRAL, and hide the PROD file) -----
setup
nl=$'\n'
mkdir -p "crates/a${nl}b/src"
printf 'pub fn x()->i32{42}\n' > "crates/a${nl}b/src/lib.rs"
printf '#[test]\nfn adds(){assert_eq!(vf_core::add(1,2),3);}\n#[test]\nfn more(){assert!(true);}\n' > crates/vf-core/tests/add.rs
commit prod-newline-dir-plus-test
check_contains "PROD file under a newline-named directory + test edit" partition fail \
  'production files: 1'

# --- 50. a rename from TEST to PROD: refused (VFL-819 LOW-3; the R record
#     yields both the old TEST path and the new PROD path) ----------------
setup
git mv crates/vf-core/tests/more.rs crates/vf-core/src/more.rs
commit rename-test-to-prod
check_contains "rename TEST file to a PROD path (tests/more.rs -> src/more.rs)" partition fail \
  'production files: 1' 'test files: 1'

# --- 51. a TEST file deleted alone: allowed (TEST-side control for case 43,
#     which is the GATE-side deletion-alone control) -----------------------
setup
rm crates/vf-core/tests/more.rs
commit delete-test-file-alone
check_contains "delete TEST file alone (crates/vf-core/tests/more.rs)" partition pass \
  'test files: 1'

# --- 52. two GATE files bundled with production source: refused, and the
#     refusal names every GATE path, not just one (VFL-843 review LOW: no
#     existing case had more than one GATE path in the diff) --------------
setup
mkdir -p .tekton/tasks
printf 'kind: Task\n' > .tekton/tasks/lane-gate.yaml
printf 'kind: Kustomization\n' > .tekton/kustomization.yaml
printf 'pub fn add(a:i32,b:i32)->i32{a+b}\npub fn sub(a:i32,b:i32)->i32{a-b}\n' > crates/vf-core/src/lib.rs
commit two-gate-files-plus-prod
check_contains "two GATE files (.tekton/tasks/lane-gate.yaml, .tekton/kustomization.yaml) + prod" partition fail \
  'gate files in this diff:' '.tekton/tasks/lane-gate.yaml' '.tekton/kustomization.yaml' \
  'production source in this diff:' 'crates/vf-core/src/lib.rs'

# --- 53. changed_files fails closed on a status it cannot parse (VFL-862
#     LOW-3): a git stub ahead of PATH returns a C100 copy record, which is
#     not R* and not one of A/D/M/T, so changed_files' case falls through to
#     its fail-closed default instead of silently skipping the record ------
setup
install_git_stub 'C100\0a\0b\0'
check_contains_stubbed_git "changed_files fails closed on an unparsed status (C100 copy record)" fail \
  'changed_files does not parse'

# --- 54. changed_files fails closed on a record cut short (VFL-862 LOW-3):
#     the stub emits a bare M status with no path behind it, so the second
#     read in changed_files' while loop hits end of stream before the path
#     it expects, instead of silently reading the next record out of step --
setup
install_git_stub 'M\0'
check_contains_stubbed_git "changed_files fails closed on a record cut short (status with no path)" fail \
  'ends early'

# --- 55. pins: a commit SHA with a trailing tag comment: allowed ---------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262 # v4.4.0\n' > .github/workflows/ci.yml
commit add-workflow-pinned
check "uses: pinned to a SHA with a tag comment" pins pass

# --- 56. pins: a tag instead of a SHA: refused ----------------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@v4\n' > .github/workflows/ci.yml
commit add-workflow-tag
check "uses: pinned to a tag (@v4)" pins fail

# --- 57. pins: a branch instead of a SHA: refused -------------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@main\n' > .github/workflows/ci.yml
commit add-workflow-branch
check "uses: pinned to a branch (@main)" pins fail

# --- 58. pins: a 39-character SHA (too short): refused --------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af67726\n' > .github/workflows/ci.yml
commit add-workflow-short-sha
check "uses: SHA is 39 hex characters" pins fail

# --- 59. pins: a 41-character SHA (too long): refused ----------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af6772622\n' > .github/workflows/ci.yml
commit add-workflow-long-sha
check "uses: SHA is 41 hex characters" pins fail

# --- 60. pins: an uppercase SHA: refused -----------------------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11D5960A326750D5838078E36CF38B85AF677262\n' > .github/workflows/ci.yml
commit add-workflow-upper-sha
check "uses: SHA is uppercase hex" pins fail

# --- 61. pins: no @ref at all: refused -------------------------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout\n' > .github/workflows/ci.yml
commit add-workflow-no-ref
check "uses: has no @ref" pins fail

# --- 62. pins: a local ./ action: refused -----------------------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: ./.github/actions/my-action\n' > .github/workflows/ci.yml
commit add-workflow-local-action
check "uses: a local ./ action" pins fail

# --- 63. pins: a docker:// image: refused -----------------------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: docker://alpine:3.18\n' > .github/workflows/ci.yml
commit add-workflow-docker-image
check "uses: a docker:// image" pins fail

# --- 64. pins: an empty value: refused --------------------------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses:\n' > .github/workflows/ci.yml
commit add-workflow-empty-value
check "uses: has an empty value" pins fail

# --- 65. pins: a block-scalar value: refused --------------------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: >-\n          actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' > .github/workflows/ci.yml
commit add-workflow-block-scalar
check "uses: a block-scalar (>-) value" pins fail

# --- 66. pins: a commented-out uses: line is ignored: allowed --------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      # - uses: actions/checkout@v4\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' > .github/workflows/ci.yml
commit add-workflow-commented-and-pinned
check "commented-out uses: line is ignored" pins pass

# --- 67. pins: a quoted key/value in a flow mapping: allowed ---------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - { "uses": "actions/checkout@11d5960a326750d5838078e36cf38b85af677262" }\n' > .github/workflows/ci.yml
commit add-workflow-quoted-flow-mapping
check "quoted uses: in a flow mapping" pins pass

# --- 68. pins: a workflow with no uses: key at all: allowed ----------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - run: echo hi\n' > .github/workflows/ci.yml
commit add-workflow-no-uses-key
check "workflow with no uses: key" pins pass

# --- 69. pins: a tree with no .github/workflows/ at all: allowed -----------
# Regression: setup() creates no .github/workflows/ today, so this is the
# ordinary state before any workflow lands, and git grep's no-match exit (1)
# must not be mistaken for a read failure.
setup
check "no .github/workflows/ anywhere in the tree" pins pass

# --- 70. all: an unpinned uses: fails the suite even though the other -----
#          three checks on this diff would pass on their own ---------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@v4\n' > .github/workflows/ci.yml
commit add-workflow-tag-via-all
check "all: unpinned uses: fails the combined run" all fail

# --- 71. pins: an unresolvable head-ref: refused ----------------------------
# check() always compares baseline against HEAD, so this case calls the gate
# directly with a ref that cannot be resolved.
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' > .github/workflows/ci.yml
commit add-workflow-for-bad-head-ref
out="$(./ci/lane-gate.sh pins baseline does-not-exist-ref 2>&1)"; rc=$?
name="unresolvable head-ref"
if [ $rc -ne 0 ]; then
  printf 'ok    %-46s %s=%s\n' "$name" pins fail; pass_count=$((pass_count+1))
else
  printf 'NOT OK %-45s %s: expected fail got pass\n' "$name" pins
  printf '%s\n' "$out" | sed 's/^/        | /'
  fail_count=$((fail_count+1))
fi

#   The cases below (72-87) are regression fixtures for VFL-122 review M1
#   (e0be0ce): `pins` used to test only the leftmost uses: on a line, so a
#   pinned decoy earlier on the line let an unpinned ref after it pass.

# --- 72. pins: a pinned decoy before an unpinned uses:, same line: refused --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps: [{uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262}, {uses: evil/act@v4}]\n' > .github/workflows/ci.yml
commit add-workflow-pinned-decoy-first
check "pinned decoy before an unpinned uses: (same line)" pins fail

# --- 73. pins: same decoy, reversed order: refused --------------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps: [{uses: evil/act@v4}, {uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262}]\n' > .github/workflows/ci.yml
commit add-workflow-pinned-decoy-second
check "pinned decoy after an unpinned uses: (same line)" pins fail

# --- 74. pins: a uses: inside a quoted string masks the real key: refused ---
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - { name: "replaces uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262", uses: actions/checkout@v4 }\n' > .github/workflows/ci.yml
commit add-workflow-uses-inside-string
check "uses: text inside a quoted string, real key unpinned" pins fail

# --- 75. pins: a plain value cut short by a comma (@<sha>,v4): refused ------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: evil/act@11d5960a326750d5838078e36cf38b85af677262,v4\n' > .github/workflows/ci.yml
commit add-workflow-comma-suffix
check "uses: value followed by ,v4 with no whitespace" pins fail

# --- 76. pins: two valid uses: values on one line: allowed ------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps: [{ uses: a/b@11d5960a326750d5838078e36cf38b85af677262 }, { uses: "c/d@11d5960a326750d5838078e36cf38b85af677262" }]\n' > .github/workflows/ci.yml
commit add-workflow-two-pinned-values
check "two valid pinned uses: values on one line" pins pass

# --- 77. pins: uses: inside a flow-sequence bracket: refused ----------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps: [uses: x/y@v4]\n' > .github/workflows/ci.yml
commit add-workflow-bracket-key
check "uses: key opening a flow-sequence bracket" pins fail

# --- 78. pins: a !!str tag before the uses: key: refused --------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - !!str uses: x/y@v4\n' > .github/workflows/ci.yml
commit add-workflow-tagged-key
check "!!str tag before an unpinned uses: key" pins fail

# --- 79. pins: an anchor (&k) before the uses: key: refused -----------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - &k uses: x/y@v4\n' > .github/workflows/ci.yml
commit add-workflow-anchored-key
check "anchor (&k) before an unpinned uses: key" pins fail

# --- 80. pins: an explicit "? uses" key, value on the next line: refused ----
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - ? uses\n        : x/y@v4\n' > .github/workflows/ci.yml
commit add-workflow-explicit-key
check "explicit ? uses key with value on the next line" pins fail

# --- 81. pins: a single-quoted value with a doubled '': refused ------------
# The doubled '' is YAML's escape for a literal quote inside a single-quoted
# scalar. SQ_VALUE_RE requires the character after the closing quote to be a
# non-quote (or end of line), so it does not match here; the scanner falls
# back to PLAIN_VALUE_RE, which cannot produce a value that satisfies PIN_RE.
# This refuses a line it cannot parse rather than guessing at its value.
setup
mkdir -p .github/workflows
printf "on: push\njobs:\n  build:\n    steps:\n      - uses: 'x/y@11d5960a326750d5838078e36cf38b85af677262''v4'\n" > .github/workflows/ci.yml
commit add-workflow-doubled-single-quote
check "single-quoted value with a doubled '' escape" pins fail

# --- 82. pins: a local ./ action pinned to a SHA: refused -------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: ./.github/actions/x@11d5960a326750d5838078e36cf38b85af677262\n' > .github/workflows/ci.yml
commit add-workflow-local-action-pinned
check "local ./ action pinned to a SHA" pins fail

# --- 83. pins: a docker:// image pinned to a SHA: refused -------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: docker://alpine@11d5960a326750d5838078e36cf38b85af677262\n' > .github/workflows/ci.yml
commit add-workflow-docker-image-pinned
check "docker:// image pinned to a SHA" pins fail

# --- 84. pins: a comment line holding a quote character is not skipped -----
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      # - uses: "x/y@v4"\n' > .github/workflows/ci.yml
commit add-workflow-commented-quoted-decoy
check "# comment holding a quote is examined, not skipped" pins fail

# --- 85. pins: an unpinned uses: under a nested workflows/sub/*.yaml -------
setup
mkdir -p .github/workflows/sub
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@v4\n' > .github/workflows/sub/ci.yaml
commit add-workflow-nested-yaml-unpinned
check "unpinned uses: under .github/workflows/sub/ci.yaml" pins fail

# --- 86. pins: a job-level reusable workflow call pinned to a SHA: allowed --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    uses: o/r/.github/workflows/x.yml@11d5960a326750d5838078e36cf38b85af677262\n' > .github/workflows/ci.yml
commit add-workflow-reusable-pinned
check "job-level reusable workflow call, pinned to a SHA" pins pass

# --- 87. pins: an unpinned uses: is still found when run from ci/ ----------
# Regression: `pins` now reads from the repository root (git rev-parse
# --show-toplevel), so invoking it with cwd=ci/ must not vacuously pass by
# looking for .github/workflows/ under ci/.github/workflows/ instead.
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@v4\n' > .github/workflows/ci.yml
commit add-workflow-tag-for-subdir-cwd
out="$(cd ci && ./lane-gate.sh pins baseline HEAD 2>&1)"; rc=$?
name="pins from ci/ subdirectory still checks .github/workflows/"
if [ $rc -ne 0 ]; then
  printf 'ok    %-46s %s=%s\n' "$name" pins fail; pass_count=$((pass_count+1))
else
  printf 'NOT OK %-45s %s: expected fail got pass\n' "$name" pins
  printf '%s\n' "$out" | sed 's/^/        | /'
  fail_count=$((fail_count+1))
fi

#   The cases below (88-95) are regression fixtures for VFL-122 review round 2
#   (1b91879): the line scanner only ever caught a `uses:` whose scope line
#   held the literal word `uses` before a colon; each form below loads as
#   `uses: evil/act@v4` in js-yaml 4.3.1 but never matched that scan, so it
#   passed `pins` on 53f326e.

# --- 88. pins: a lone CR after a comment hides a step: refused -------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      # c\r      - uses: evil/act@v4\n' > .github/workflows/ci.yml
commit add-workflow-lone-cr-after-comment
check "lone CR after a comment hides an unpinned uses:" pins fail

# --- 89. pins: an explicit key spelled as a block scalar: refused ----------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - ? >-\n          uses\n        : evil/act@v4\n' > .github/workflows/ci.yml
commit add-workflow-explicit-key-block-scalar
check "explicit ? key spelled as a block scalar" pins fail

# --- 90. pins: a double-quoted key holding a backslash escape: refused -----
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - "u\\x73es": evil/act@v4\n' > .github/workflows/ci.yml
commit add-workflow-escaped-key
check "double-quoted key holding a backslash escape" pins fail

# --- 91. pins: an alias used as a key: refused ------------------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - x: &k uses\n      - *k : evil/act@v4\n' > .github/workflows/ci.yml
commit add-workflow-alias-key
check "alias (*k) used as a key" pins fail

# --- 92. pins: a UTF-16 workflow file: refused ------------------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: evil/act@v4\n' | iconv -f utf-8 -t utf-16 > .github/workflows/ci.yml
commit add-workflow-utf16
check "UTF-16 workflow file (git sees it as binary)" pins fail

# --- 93. pins: the workflow file is a symlink to content outside .github/: refused --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: evil/act@v4\n' > outside-workflow.yml
ln -s ../../outside-workflow.yml .github/workflows/ci.yml
commit add-workflow-symlink-to-outside-file
check "workflow file is a symlink to a file outside .github/" pins fail

# --- 94. pins: a CRLF workflow, pinned uses: plus a commented-out uses:: allowed --
setup
mkdir -p .github/workflows
printf 'on: push\r\njobs:\r\n  build:\r\n    steps:\r\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\r\n      # - uses: x/y@v4\r\n' > .github/workflows/ci.yml
commit add-workflow-crlf-pinned-and-commented
check "CRLF workflow: pinned uses: plus a commented-out uses:" pins pass

# --- 95. pins: a run: block holding *.rs, a ?) case arm and \n: allowed ----
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n      - run: |\n          ls *.rs\n          case $x in ?) ;; esac\n          printf "%%s\\n" "$x"\n' > .github/workflows/ci.yml
commit add-workflow-run-block-shell-metachars
check "run: block holding *.rs, a ?) case arm and a backslash escape" pins pass

#   The cases below (96-106) are discretionary extras for the same VFL-122
#   review round (1b91879): flow-form and symlink/submodule variants the
#   architect called out as optional, plus a few boundary checks confirming
#   the new refusals don't overreach onto lines that were always fine.

# --- 96. pins: a NEL line break after a comment hides a step: refused ------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      # c\xc2\x85      - uses: evil/act@v4\n' > .github/workflows/ci.yml
commit add-workflow-nel-after-comment
check "NEL line break after a comment hides an unpinned uses:" pins fail

# --- 97. pins: a LS line break after a comment hides a step: refused -------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      # c\xe2\x80\xa8      - uses: evil/act@v4\n' > .github/workflows/ci.yml
commit add-workflow-ls-after-comment
check "LS line break after a comment hides an unpinned uses:" pins fail

# --- 98. pins: a flow explicit key, { ? uses : v }: refused ----------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - { ? uses : evil/act@v4 }\n' > .github/workflows/ci.yml
commit add-workflow-flow-explicit-key
check "flow explicit { ? uses : ... } key" pins fail

# --- 99. pins: a double-quoted "uses" key with no escape, unpinned: refused --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - "uses": evil/act@v4\n' > .github/workflows/ci.yml
commit add-workflow-quoted-uses-key-unpinned
check "quoted \"uses\" key with no escape, unpinned value" pins fail

# --- 100. pins: a flow alias used as a key, {*k : v}: refused ---------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - x: &k uses\n      - {*k : evil/act@v4}\n' > .github/workflows/ci.yml
commit add-workflow-flow-alias-key
check "flow {*k : ...} alias key" pins fail

# --- 101. pins: .github itself is a symlink: refused -------------------------
setup
mkdir -p real-github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' > real-github/workflows/ci.yml
ln -s real-github .github
commit add-github-dir-symlink
check ".github itself is a symlink" pins fail

# --- 102. pins: .github/workflows itself is a symlink: refused ---------------
setup
mkdir -p .github real-workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' > real-workflows/ci.yml
ln -s ../real-workflows .github/workflows
commit add-workflows-dir-symlink
check ".github/workflows itself is a symlink" pins fail

# --- 103. pins: a gitlink (submodule) under .github/workflows/: refused -----
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' > .github/workflows/ci.yml
git add -A >/dev/null
git update-index --add --cacheinfo 160000,11d5960a326750d5838078e36cf38b85af677262,.github/workflows/sub
git commit -qm add-workflows-gitlink
check "gitlink (submodule) under .github/workflows/" pins fail

# --- 104. pins: a quote-less comment holding ?, * and : beside a pinned step: allowed --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n      # what? yes, ? maybe *k : no\n' > .github/workflows/ci.yml
commit add-workflow-quoteless-comment-with-triggers
check "quote-less comment holding ?, * and : beside a pinned step" pins pass

# --- 105. pins: an empty workflow file beside a pinned one: allowed ---------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' > .github/workflows/ci.yml
: > .github/workflows/empty.yml
commit add-empty-workflow-file
check "empty workflow file beside a pinned one" pins pass

# --- 106. pins: an executable (100755) workflow file: allowed ---------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' > .github/workflows/ci.yml
chmod +x .github/workflows/ci.yml
commit add-executable-workflow
check "executable (100755) workflow file" pins pass

#   The cases below (107-111) are regression fixtures for VFL-122 review round 2
#   (c977082, M2): split_hit splits each `git grep -n` line on the first two
#   colons after the ref, so a colon in the path moved the rest of the path
#   into the line text, and when that tail began with `#` the line passed for
#   a comment and was skipped. .github/workflows/a:b:#.yml holding
#   `uses: evil/act@v4` gave `PASS pins: no uses: references`, exit 0, on
#   c977082. 68f37be refuses any path under .github/workflows/ that holds a
#   colon, and case 110 closes the PS (U+2029) fixture gap the same review (L8)
#   found alongside it.

# --- 107. pins: a workflow path holding a colon, unpinned uses: (M2 reproduction): refused --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: evil/act@v4\n' > ".github/workflows/a:b:#.yml"
commit add-workflow-colon-path-unpinned
check "workflow path holding a colon, unpinned uses: (M2 reproduction)" pins fail

# --- 108. pins: same colon path, a correctly pinned uses:: refused by the path, by design --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' > ".github/workflows/a:b:#.yml"
commit add-workflow-colon-path-pinned
check "workflow path holding a colon, correctly pinned uses: (refused by the path)" pins fail

# --- 109. pins: a directory holding a colon under .github/workflows/: refused --
setup
mkdir -p ".github/workflows/d:x:#"
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: evil/act@v4\n' > ".github/workflows/d:x:#/w.yml"
commit add-workflow-colon-dir
check "directory holding a colon under .github/workflows/" pins fail

# --- 110. pins: a PS (U+2029) line break after a comment hides a step: refused --
# No existing fixture exercised PS; BREAK_RE already matches it (cases 88, 96
# and 97 cover CR, NEL and LS), so this closes a coverage gap and is not a
# claim that 68f37be changed this behaviour.
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      # c\xe2\x80\xa9      - uses: evil/act@v4\n' > .github/workflows/ci.yml
commit add-workflow-ps-after-comment
check "PS (U+2029) line break after a comment hides an unpinned uses:" pins fail

# --- 111. pins: a quoted path holding a double quote but no colon: allowed --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n      # comment\n' > '.github/workflows/q"x.yml'
commit add-workflow-quoted-path-no-colon
check "quoted path holding a double quote but no colon" pins pass

#   The cases below (112-116) are discretionary extras for the same review round
#   (VFL-472): path-colon boundary variants the task called out as optional.

# --- 112. pins: a:b: #.yml (space before the hash) path, unpinned uses:: refused --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: evil/act@v4\n' > ".github/workflows/a:b: #.yml"
commit add-workflow-colon-space-hash-path
check "workflow path a:b: #.yml (space before the hash), unpinned uses:" pins fail

# --- 113. pins: a colon-holding path is still refused when run from ci/ ----
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: evil/act@v4\n' > ".github/workflows/a:b:#.yml"
commit add-workflow-colon-path-for-subdir-cwd
out="$(cd ci && ./lane-gate.sh pins baseline HEAD 2>&1)"; rc=$?
name="pins from ci/ subdirectory still refuses a colon-holding path"
if [ $rc -ne 0 ]; then
  printf 'ok    %-46s %s=%s\n' "$name" pins fail; pass_count=$((pass_count+1))
else
  printf 'NOT OK %-45s %s: expected fail got pass\n' "$name" pins
  printf '%s\n' "$out" | sed 's/^/        | /'
  fail_count=$((fail_count+1))
fi

# --- 114. pins: a colon-holding path with no uses: key at all: refused -----
setup
mkdir -p .github/workflows
printf 'just plain text, no uses key here\n' > ".github/workflows/notes:x"
commit add-workflow-colon-path-no-uses
check "colon-holding workflow path with no uses: key at all" pins fail

# --- 115. pins: a non-ASCII path (quoted by git), no colon: allowed --------
setup
mkdir -p .github/workflows
F=$'.github/workflows/\xc3\xa9.yml'
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n      # comment\n' > "$F"
commit add-workflow-nonascii-path
check "non-ASCII workflow path (quoted by git), no colon" pins pass

# --- 116. pins: a pinned uses: value separated by a literal TAB: allowed ---
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  build:\n    steps:\n      - uses:\tactions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' > .github/workflows/ci.yml
commit add-workflow-tab-after-uses-colon
check "pinned uses: value separated by a literal TAB" pins pass

#   The cases below (117-125) are regression fixtures for VFL-122 review round 3
#   (c74e16e, M3): the scanner assumes a key and its colon share a line, which
#   holds in block context but not in a flow mapping, where a line break or a
#   comment may come between an implicit key and its colon.
#   `steps: [ { name: checkout, uses` then `: evil/act@v4, with: { ref: main } } ]`
#   on the next line gave PASS, exit 0, on ced2971, while GitHub's own
#   @actions/workflow-parser read it as uses: evil/act@v4 with no errors.
#   c74e16e refuses a literal, escaped or alias key whose colon sits on a
#   later line, a double-quoted key continued by an escaped line break, and a
#   backslash in a pinned path (which a double-quoted value would decode).

# --- 117. pins: a flow step uses key, colon on a later line (M3 reproduction): refused --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps: [ { name: checkout, uses\n      : evil/act@v4, with: { ref: main } } ]\n' > .github/workflows/ci.yml
commit add-workflow-flow-split-key-with-trailer
check "flow step uses key with its colon on a later line (M3 reproduction)" pins fail

# --- 118. pins: a quoted "uses" key, colon on a later line: refused ---------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps: [ { "uses"\n      : evil/act@v4 } ]\n' > .github/workflows/ci.yml
commit add-workflow-quoted-split-key
check "quoted \"uses\" key with its colon on a later line" pins fail

# --- 119. pins: a job-level uses key, colon on a later line: refused --------
setup
mkdir -p .github/workflows
printf 'on: push\njobs: { b: { uses\n  : evil/wf/.github/workflows/w.yml@main } }\n' > .github/workflows/ci.yml
commit add-workflow-job-level-split-key
check "job-level uses key with its colon on a later line" pins fail

# --- 120. pins: a double-quoted key continued by an escaped line break: refused --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps: [ { "us\\\n      es": evil/act@v4 } ]\n' > .github/workflows/ci.yml
commit add-workflow-escaped-key-line-break
check "double-quoted key continued by an escaped line break" pins fail

# --- 121. pins: an escaped "uses" key, colon on a later line: refused ------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps: [ { "u\\x73es"\n      : evil/act@v4 } ]\n' > .github/workflows/ci.yml
commit add-workflow-escaped-key-colon-split
check "escaped \"uses\" key with its colon on a later line" pins fail

# --- 122. pins: a backslash in a pinned path: refused -----------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: "evil/act/p\\x40v4@11d5960a326750d5838078e36cf38b85af677262"\n' > .github/workflows/ci.yml
commit add-workflow-backslash-in-pinned-path
check "backslash in a pinned path (a double-quoted value would decode it)" pins fail

# --- 123. pins: a pinned flow step, key colon and value on one line: allowed --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps: [ { uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262 } ]\n' > .github/workflows/ci.yml
commit add-workflow-flow-step-one-line-pinned
check "pinned flow step, key, colon and value all on one line" pins pass

# --- 124. pins: a flow step split across lines, key+colon+value together: allowed --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps: [ { name: a,\n      uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262 } ]\n' > .github/workflows/ci.yml
commit add-workflow-flow-step-split-colon-with-value
check "flow step split across lines, with key, colon and value together" pins pass

# --- 125. pins: a pinned step beside a run: printf holding an escaped quote: allowed --
# Confirms the escaped-key and split-key rules do not reach into a run: block:
# the closed, non-flow-key-position quote in printf "%s\n" a is not mistaken
# for a key.
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n      - run: printf "%%s\\n" a\n' > .github/workflows/ci.yml
commit add-workflow-run-printf-escaped-quote
check "pinned step beside a run: printf holding an escaped quote" pins pass

#   The cases below (126-130) are discretionary extras for the same review round
#   (c74e16e, M3): boundary variants the architect called out as optional.

# --- 126. pins: a split uses key followed by a comment on its own line: refused --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps: [ { uses # pick one\n      : evil/act@v4 } ]\n' > .github/workflows/ci.yml
commit add-workflow-split-key-comment-after
check "split uses key followed by a comment before the colon" pins fail

# --- 127. pins: a split uses key with a comment line between key and colon: refused --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps: [ { uses\n      # pick one\n      : evil/act@v4 } ]\n' > .github/workflows/ci.yml
commit add-workflow-split-key-comment-between
check "split uses key with a comment line between the key and the colon" pins fail

# --- 128. pins: a split uses key after an anchor: refused -------------------
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps: [ { &a uses\n      : evil/act@v4 } ]\n' > .github/workflows/ci.yml
commit add-workflow-split-key-after-anchor
check "split uses key after an anchor" pins fail

# --- 129. pins: a job-level split key, CRLF line ends: refused --------------
setup
mkdir -p .github/workflows
printf 'on: push\r\njobs: { b: { uses\r\n  : evil/wf/.github/workflows/w.yml@main } }\r\n' > .github/workflows/ci.yml
commit add-workflow-job-level-split-key-crlf
check "job-level split key, CRLF line ends" pins fail

# --- 130. pins: a pinned step beside a run: echo ending in the word uses: refused --
# Accepted over-refusal (the same trade as case 125/L9): prose or shell that
# ends a line in the word uses is refused too, even though it is not a key.
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n      - run: echo "this step uses"\n' > .github/workflows/ci.yml
commit add-workflow-run-echo-this-step-uses
check "pinned step beside a run: echo line ending in the word uses" pins fail

#   The cases below (131-137) are for the same VFL-122 review round (c1a46cf,
#   M4): the non-text pass now decides from the blob, not from git's diff
#   attribute, which a tracked .gitattributes in the tree under check can set.

# --- 131. pins: a UTF-16 workflow is refused despite a root .gitattributes *.yml diff (H1): refused --
setup
mkdir -p .github/workflows
printf '*.yml diff\n' > .gitattributes
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: evil/act@v4\n' | iconv -f utf-8 -t utf-16 > .github/workflows/ci.yml
commit add-workflow-utf16-root-gitattributes-diff
check "UTF-16 workflow refused despite root .gitattributes *.yml diff (H1)" pins fail

# --- 132. pins: a UTF-16 workflow is refused despite .github/workflows/.gitattributes *.yml diff (H2): refused --
setup
mkdir -p .github/workflows
printf '*.yml diff\n' > .github/workflows/.gitattributes
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: evil/act@v4\n' | iconv -f utf-8 -t utf-16 > .github/workflows/ci.yml
commit add-workflow-utf16-workflows-gitattributes-diff
check "UTF-16 workflow refused despite .github/workflows/.gitattributes *.yml diff (H2)" pins fail

# --- 133. pins: a pinned UTF-16 workflow is refused too, the blob decides whatever the value says: refused --
setup
mkdir -p .github/workflows
printf '*.yml diff\n' > .gitattributes
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' | iconv -f utf-8 -t utf-16 > .github/workflows/ci.yml
commit add-workflow-utf16-pinned-root-gitattributes-diff
check "pinned UTF-16 workflow refused by the blob NUL check regardless of the pinned value" pins fail

# --- 134. pins: a pinned UTF-8 workflow is refused when a root .gitattributes *.yml -diff makes git grep see it as binary: refused --
setup
mkdir -p .github/workflows
printf '*.yml -diff\n' > .gitattributes
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' > .github/workflows/ci.yml
commit add-workflow-pinned-root-gitattributes-nodiff
check "pinned UTF-8 workflow refused when *.yml -diff makes git grep report it as binary" pins fail

# --- 135. pins: an unrelated *.md text attribute leaves a pinned UTF-8 workflow passing: allowed --
setup
mkdir -p .github/workflows
printf '*.md text\n' > .gitattributes
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' > .github/workflows/ci.yml
commit add-workflow-pinned-root-gitattributes-md-text
check "unrelated .gitattributes (*.md text) leaves a pinned UTF-8 workflow passing" pins pass

# --- 136. pins: a pinned UTF-8 workflow still passes with a root .gitattributes *.yml diff: allowed --
setup
mkdir -p .github/workflows
printf '*.yml diff\n' > .gitattributes
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' > .github/workflows/ci.yml
commit add-workflow-pinned-root-gitattributes-diff
check "pinned UTF-8 workflow still passes with root .gitattributes *.yml diff" pins pass

# --- 137. pins: a NUL-holding .github/logo.png beside a pinned workflow is not read: allowed --
setup
mkdir -p .github/workflows
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' > .github/workflows/ci.yml
printf 'a\0b' > .github/logo.png
commit add-workflow-pinned-plus-logo-with-nul
check "pinned workflow passes beside a NUL-holding .github/logo.png (outside workflows/)" pins pass

#   The cases below (138-140) are discretionary extras for the same review
#   round (c1a46cf, M4): further encodings and a boundary probe the architect
#   called out as optional.

# --- 138. pins: a UTF-32 workflow is refused despite a root .gitattributes *.yml diff: refused --
setup
mkdir -p .github/workflows
printf '*.yml diff\n' > .gitattributes
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: evil/act@v4\n' | iconv -f utf-8 -t utf-32 > .github/workflows/ci.yml
commit add-workflow-utf32-root-gitattributes-diff
check "UTF-32 workflow refused despite root .gitattributes *.yml diff" pins fail

# --- 139. pins: a pinned UTF-8 workflow with one NUL after byte 8000 is refused: refused --
setup
mkdir -p .github/workflows
{
  printf '# '
  printf 'x%.0s' $(seq 1 8010)
  printf '\n'
  printf '\0'
  printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n'
} > .github/workflows/ci.yml
commit add-workflow-pinned-nul-after-byte-8000
check "pinned UTF-8 workflow with one NUL after byte 8000 is refused" pins fail

# --- 140. pins: an unpinned UTF-8 workflow is refused when a root .gitattributes *.yml binary makes git grep see it as binary: refused --
setup
mkdir -p .github/workflows
printf '*.yml binary\n' > .gitattributes
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: evil/act@v4\n' > .github/workflows/ci.yml
commit add-workflow-unpinned-root-gitattributes-binary
check "unpinned UTF-8 workflow refused when *.yml binary makes git grep report it as binary" pins fail

#   The cases below (141-144) are regression fixtures for VFL-122 review round 5
#   (6cac319, CodeRabbit VFL-540 HIGH / Opus Reviewer VFL-541 M5): ls-tree
#   C-quotes a path holding a byte above 0x7f, a double quote, a backslash or
#   a control character. A quoted path starts with `"`, missed the
#   .github/workflows/* pattern below, and its blob was never read, so a
#   UTF-16 or UTF-32 workflow at such a path passed with an unpinned uses: on
#   6cac319. 0068b1c reads both ls-tree listings with -z so every path
#   matches as stored, whatever bytes it holds; none of these fixtures sets
#   core.quotePath, so git's default (quoting on) stands.

# --- 141. pins: a UTF-16 unpinned workflow at a non-ASCII filename (M5 reproduction): refused --
setup
mkdir -p .github/workflows
F=$'.github/workflows/\xc3\xa9.yml'
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: evil/act@v4\n' | iconv -f utf-8 -t utf-16 > "$F"
commit add-workflow-utf16-nonascii-filename
check "UTF-16 unpinned workflow at a non-ASCII filename (M5 reproduction)" pins fail

# --- 142. pins: a UTF-16 unpinned workflow at a filename holding a double quote: refused --
setup
mkdir -p .github/workflows
F='.github/workflows/a"b.yml'
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: evil/act@v4\n' | iconv -f utf-8 -t utf-16 > "$F"
commit add-workflow-utf16-quote-filename
check "UTF-16 unpinned workflow at a filename holding a double quote" pins fail

# --- 143. pins: a UTF-16 unpinned workflow under a non-ASCII directory: refused --
setup
D=$'.github/workflows/d\xc3\xa9'
mkdir -p "$D"
F="$D/ci.yml"
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: evil/act@v4\n' | iconv -f utf-8 -t utf-16 > "$F"
commit add-workflow-utf16-nonascii-directory
check "UTF-16 unpinned workflow under a non-ASCII directory" pins fail

# --- 144. pins: a pinned UTF-8 workflow at a non-ASCII filename still passes: allowed --
# Guards against a fix that refuses every quoted path instead of reading its blob.
setup
mkdir -p .github/workflows
F=$'.github/workflows/\xc3\xa9.yml'
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' > "$F"
commit add-workflow-pinned-nonascii-filename
check "pinned UTF-8 workflow at a non-ASCII filename passes" pins pass

#   The cases below (145-148) are discretionary extras for the same review
#   round (6cac319/0068b1c, M5): further encodings and quoted-byte variants
#   the fix commit's own probe list called out, plus a pass-side boundary
#   check pairing with case 142.

# --- 145. pins: a UTF-32 unpinned workflow at a non-ASCII filename: refused --
setup
mkdir -p .github/workflows
F=$'.github/workflows/\xc3\xa9.yml'
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: evil/act@v4\n' | iconv -f utf-8 -t utf-32 > "$F"
commit add-workflow-utf32-nonascii-filename
check "UTF-32 unpinned workflow at a non-ASCII filename" pins fail

# --- 146. pins: a UTF-16 unpinned workflow at a filename holding a backslash: refused --
setup
mkdir -p .github/workflows
F='.github/workflows/a\b.yml'
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: evil/act@v4\n' | iconv -f utf-8 -t utf-16 > "$F"
commit add-workflow-utf16-backslash-filename
check "UTF-16 unpinned workflow at a filename holding a backslash" pins fail

# --- 147. pins: a UTF-16 unpinned workflow at a filename holding a tab: refused --
setup
mkdir -p .github/workflows
F=$'.github/workflows/a\tb.yml'
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: evil/act@v4\n' | iconv -f utf-8 -t utf-16 > "$F"
commit add-workflow-utf16-tab-filename
check "UTF-16 unpinned workflow at a filename holding a tab" pins fail

# --- 148. pins: a pinned UTF-8 workflow at a filename holding a double quote still passes: allowed --
setup
mkdir -p .github/workflows
F='.github/workflows/a"b.yml'
printf 'on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n' > "$F"
commit add-workflow-pinned-quote-filename
check "pinned UTF-8 workflow at a filename holding a double quote passes" pins pass

printf '\n%s passed, %s failed\n' "$pass_count" "$fail_count"
[ "$fail_count" -eq 0 ]
