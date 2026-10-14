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

printf '\n%s passed, %s failed\n' "$pass_count" "$fail_count"
[ "$fail_count" -eq 0 ]
