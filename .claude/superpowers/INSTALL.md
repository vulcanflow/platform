# Superpowers — vendored install in `vulcanflow/platform`

Upstream: https://github.com/obra/superpowers (MIT — see [`LICENSE`](./LICENSE))

| | |
|---|---|
| Version | 6.4.2 |
| Commit | `8ca22dba9a94f28898bbce59f2537ff4d87c747d` |
| Tag | `v6.4.2` → the same commit |
| Upstream commit date | 2026-09-25 |
| Vendored | 2026-10-01 |
| Skills | 15 upstream + 1 vulcanFlow adapter, in `.claude/skills/` |
| Files | 64 under `.claude/` — 62 skill files, `vulcanflow-superpowers/SKILL.md`, this `LICENSE` |
| Decision | [ADR-0007](https://github.com/vulcanflow/docs/blob/main/decisions/ADR-0007-repo-level-agent-process-bootstrap.md) |
| Issue | VUL-24 |

The pin was read from upstream at the tagged release: `git rev-parse v6.4.2^{commit}` and
`git rev-parse HEAD` both return `8ca22dba9a94f28898bbce59f2537ff4d87c747d`, and
`package.json` at that commit reports `"version": "6.4.2"`.

## Why vendored, and not the plugin install

Upstream's Claude Code path is `/plugin install superpowers@claude-plugins-official`, an
interactive slash command that writes to the *user's* `~/.claude` plugin config. That does
not work here:

- Paperclip agent runs are headless — there is no interactive `/plugin` prompt.
- A user-level plugin install is per-machine, so it does not reach agents running in other
  containers and does not survive workspace re-provisioning.

Vendoring into the repository reaches every session rooted at this checkout — agent,
subagent, or human — with no setup step, and is versioned with the code it governs.

## What is here

- `.claude/skills/<skill>/` — the 15 upstream skills and the vulcanFlow adapter
  `vulcanflow-superpowers`, including `references/`, `prompts/` and `templates/` subtrees.
- `.claude/superpowers/LICENSE` — upstream's MIT licence, copied verbatim.
- `/CLAUDE.md` — the bootstrap. It states precedence, the standing brainstorm-first rule,
  the subagent-inheritance rule and the lane table, and it imports
  `using-superpowers/SKILL.md` and `vulcanflow-superpowers/SKILL.md`. **Without it the
  skills sit on disk and never fire.**

## What is deliberately *not* here

### The executable helpers — 12 files

Paperclip does not install externally sourced executables into an agent runtime, so five
skills are missing their helpers. Each one carries a `## Paperclip note — helper scripts not
shipped` section naming its own omissions. Where a step names a missing script, do the step
by hand; no workflow depends on one.

| Skill | Omitted |
|---|---|
| `brainstorming` | `scripts/helper.js`, `scripts/server.cjs`, `scripts/start-server.sh`, `scripts/stop-server.sh` |
| `executing-plans` | `scripts/task-done`, `scripts/task-start` |
| `subagent-driven-development` | `scripts/review-package`, `scripts/sdd-workspace`, `scripts/task-brief` |
| `systematic-debugging` | `condition-based-waiting-example.ts`, `find-polluter.sh` |
| `writing-skills` | `render-graphs.js` |

`brainstorming/scripts/frame-template.html` and `writing-skills/graphviz-conventions.dot`
**are** present: they are inert data, not executables, and the company skill catalog ships
them too.

No file under `.claude/` carries the executable bit. That is checkable:
`find .claude -type f -perm -u+x` must return nothing.

### A hook

Upstream ships a `SessionStart` hook, and the Onboarding workspace install kept a copy of
it. It is not here. A hook is an executable, a project hook needs a trust prompt on first
interactive use, and the `CLAUDE.md` import achieves the same bootstrap with no executable
and no prompt. Keeping the whole of `.claude/` markdown-and-data-only is the point.

Do not add hooks to `.claude/settings.local.json` either — that file is written by the
Paperclip harness and is `.gitignore`d for that reason.

## The bytes match the control plane

This tree is **byte-identical to what Paperclip's skill catalog materialises**, which is the
property that makes drift between the two mechanisms mechanically detectable rather than a
matter of opinion (ADR-0007 §5).

- The 9 company-adapted skills — `brainstorming`, `executing-plans`,
  `subagent-driven-development`, `systematic-debugging`, `test-driven-development`,
  `using-git-worktrees`, `verification-before-completion`, `writing-plans`,
  `writing-skills` — are copied from the catalog. Each is upstream verbatim plus an appended
  `## Paperclip note` and/or `## vulcanFlow lane note`, and nothing else.
- The 6 skills.sh-sourced skills — `using-superpowers`, `diagnosing-superpowers`,
  `dispatching-parallel-agents`, `finishing-a-development-branch`, `receiving-code-review`,
  `requesting-code-review` — are byte-identical to upstream at the pin, verified against
  both the clone and the runtime cache. They carry no vulcanFlow annotation; the
  reconciliation for all six lives in `vulcanflow-superpowers` §4.
- `vulcanflow-superpowers` is vulcanFlow-authored. **This repository is its source of
  truth**: it is amended on a `platform` pull request through lane 6, and the catalog entry
  is re-synced from the merged copy. ADR-0007 §7.

To verify the upstream half:

```bash
git clone https://github.com/obra/superpowers.git /tmp/sp
git -C /tmp/sp checkout 8ca22dba9a94f28898bbce59f2537ff4d87c747d
for s in using-superpowers diagnosing-superpowers dispatching-parallel-agents \
         finishing-a-development-branch receiving-code-review requesting-code-review; do
  diff -r "/tmp/sp/skills/$s" ".claude/skills/$s" || echo "DRIFT: $s"
done
```

The 9 annotated skills differ from upstream only after their last upstream heading; the
`## Paperclip note` / `## vulcanFlow lane note` sections are the entire delta.

## Updating

An update is a `platform` pull request through lane 6 like any other change, and it must
move both mechanisms together — otherwise the byte-identity above silently breaks.

```bash
git clone https://github.com/obra/superpowers.git /tmp/sp
git -C /tmp/sp checkout <new-commit>
# Re-copy the 6 unannotated skills verbatim.
# Re-copy the 9 annotated ones, then re-apply each skill's trailing note sections.
# Drop the 12 executable helpers again (and any new one the release adds).
```

Then:

1. Refresh the version table above and the pin quoted in `/CLAUDE.md` §6.
2. Re-check the installed-skill list in `/CLAUDE.md` §6 — a release may add or remove a skill.
3. Re-read upstream's `README.md`. If their install story changes — a non-interactive plugin
   install, for instance — that is ADR-0007 revisit trigger **R3**.
4. Re-sync the company skill catalog to the same commit, so both mechanisms stay on one pin.
5. Re-run the two mechanical checks: no executable bit anywhere under `.claude/`, and every
   skill directory's frontmatter `name` equal to its directory name.

## Verified at vendoring time — 2026-10-01

Checked mechanically on the branch before the pull request opened:

- 16 skill directories; 64 files under `.claude/`; no executable bit on any of them.
- All 12 upstream executable helpers absent.
- Frontmatter `name` equals the directory name for 16/16, and every `SKILL.md` has a
  `description`.
- `ci/lane-gate.sh all` passes: `.claude/**` and `CLAUDE.md` classify as NEUTRAL, so the
  diff crosses no lane. No `.rs` file is added, so `erosion` sees no change in test count.

Live skill discovery cannot be confirmed by the session that writes the files — a session's
skill list is fixed before the commit lands. It is confirmed by the next session rooted at
this checkout, which is the closing step of VUL-24.
