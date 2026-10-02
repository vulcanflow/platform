# vulcanFlow `platform` — agent instructions

Every session rooted at this repository reads this file: agent, subagent, or human. It is
checked in deliberately, so the rules below reach you without any control-plane setup and
survive workspace re-provisioning. See [ADR-0007](https://github.com/vulcanflow/docs/blob/main/decisions/ADR-0007-repo-level-agent-process-bootstrap.md).

---

## 1. Precedence

When two sources disagree, the higher one wins. Say which one you applied.

1. The **ADR series** and **TDD v2.3** in `vulcanflow/docs` — the design of record.
2. Your **`AGENTS.md`** — your lane and its prohibitions.
3. **`vulcanflow-superpowers`** — the vulcanFlow reading of a Superpowers workflow.
4. This file.
5. The **Superpowers skill text** itself.

A skill never authorises you to cross a lane boundary. If a skill tells you to do
something your `AGENTS.md` forbids and `vulcanflow-superpowers` does not cover the case,
do not resolve it yourself and do not quietly take the lane-violating branch: say so in a
comment on the issue and route it to Atlas, who owns ADR-0005.

---

## 2. Standing board instruction — brainstorm before you plan

**Always run a brainstorming session before writing a plan.** No plan document,
implementation plan, roadmap, or task breakdown gets written until the `brainstorming`
skill has run and its design has been confirmed. This applies to every planning artifact,
including ones that look small or obvious.

- **Entering plan mode counts as planning — brainstorm first.**
- Being asked directly for "a plan" is not permission to skip to `writing-plans`. Run
  `brainstorming`, get agreement on the design, then write the plan from it.
- The only exception is an explicit, in-the-moment instruction from the board to skip
  brainstorming.

Board instruction, 2026-10-01.

---

## 3. Invoke the skills

**Before any response or action — including clarifying questions, reading the codebase, or
checking a file — invoke the skills that apply.** If you think there is even a 1% chance a
skill applies, invoke it. Announce `Using [skill] to [purpose]` and follow it. If it has a
checklist, make a todo per item.

| Trigger | Skill, first |
|---|---|
| "Build X", "add Y", "change behaviour" | `brainstorming` |
| Anything that produces a plan, including plan mode | `brainstorming`, then `writing-plans` |
| "Fix this bug", a failing test, unexpected behaviour | `systematic-debugging` |
| "Here is a spec, implement it" | `writing-plans`, then `executing-plans` |
| About to say *done*, *fixed*, *passing*, or open a PR | `verification-before-completion` |

"This is simple", "I need context first", "the skill is overkill" and "I remember this
skill" are rationalisations. Skills evolve; read the current one. You may conclude a skill
is wrong for the situation — but you must have looked.

---

## 4. A subagent you dispatch is you

**It inherits your lane and every prohibition in your `AGENTS.md`.** Delegating a
prohibited act does not launder it.

`dispatching-parallel-agents`, `subagent-driven-development` and `executing-plans` all
encourage farming work out to subagents, and none of them knows about lanes. So,
concretely:

- A coding agent (Forge, Anvil, Kiln) may **not** dispatch a subagent to create, edit,
  delete, rename, skip or `#[ignore]` a test, a fixture, a golden file, a snapshot or an
  assertion.
- A test author (Scribe, Ledger) may **not** dispatch a subagent to run a suite or to open
  production source.
- Nobody but Crucible may dispatch a subagent that runs a suite or merges.

If a plan step needs a lane you do not hold, stop and hand off to the agent who holds it.
That is a comment on the issue, not a subagent.

---

## 5. The lanes, in one table

ADR-0005. No agent holds two adjacent lanes on the same change.

| # | Lane | Owner |
|---|---|---|
| 1 | Spec — a §25 test identifier and an acceptance statement, before anything is written | Atlas |
| 2 | Write tests, from the spec, before the code | Scribe (unit/property/fuzz) · Ledger (integration/conformance/e2e) |
| 3 | Write production code until the named identifiers pass | Forge · Anvil · Kiln |
| 4 | Run the suites and publish the PASS / FAIL / MISSING ledger | Crucible — sole executor |
| 5 | A red test is a code defect: route back to lane 3 | Forge · Anvil · Kiln |
| 6 | Review — Assay by hand **and** Warren via CodeRabbit; both must approve | Assay · Warren |
| 7 | Merge to the default branch on a green suite plus two approvals | Crucible |

- **A failing test is never edited to make it green.** The one exception: a test can assert
  behaviour the TDD never promised. That is a spec change, not a test fix — Atlas amends
  the TDD or files an ADR first, and only then does the *original test author* rewrite the
  test, on its own pull request carrying no production code.
- Unit tests live in `crates/<crate>/tests/` and are written against the public API. No
  `#[cfg(test)]` modules in production source — an inline test module makes one file both
  production and test at once, which is what `ci/lane-gate.sh` cannot partition.
- The lanes are enforced mechanically by `ci/lane-gate.sh` (`partition`, `erosion`,
  `inline-tests`), not by `CODEOWNERS` — there is one shared GitHub identity, so approvals
  cannot tell Scribe from Forge. ADR-0005 §3.1.

Read `vulcanflow-superpowers` for the per-lane reading of each Superpowers skill. It names
every point where a Superpowers workflow assumes you hold the next lane and must yield.

---

## 6. Superpowers is installed here as project skills

The [Superpowers](https://github.com/obra/superpowers) library — 15 skills, pinned to
upstream commit `8ca22dba9a94f28898bbce59f2537ff4d87c747d` (v6.4.2) — is vendored in
`.claude/skills/`, alongside the vulcanFlow adapter `vulcanflow-superpowers`.

**Skill-name note.** Upstream ships as a Claude Code *plugin*, so its skills cross-reference
each other as `superpowers:<name>`. Here they are **project skills**, invoked by bare name:
read `superpowers:brainstorming` as `brainstorming`, `superpowers:test-driven-development`
as `test-driven-development`, and so on.

Installed: `brainstorming`, `diagnosing-superpowers`, `dispatching-parallel-agents`,
`executing-plans`, `finishing-a-development-branch`, `receiving-code-review`,
`requesting-code-review`, `subagent-driven-development`, `systematic-debugging`,
`test-driven-development`, `using-git-worktrees`, `using-superpowers`,
`verification-before-completion`, `writing-plans`, `writing-skills` — plus
`vulcanflow-superpowers`.

Five skills ship executable helpers upstream; those files are **not** here, because
Paperclip does not install externally sourced executables into an agent runtime. Each
affected skill carries a note saying which files are missing. Where a step names one, do
that step by hand. No workflow depends on one.

Version, pin and update procedure: [`.claude/superpowers/INSTALL.md`](.claude/superpowers/INSTALL.md).

**Availability is not permission.** A skill being on disk here does not grant you the lane
it assumes. `test-driven-development` is a live example: it is present for everyone because
a vendored directory cannot be scoped per agent, but its first instruction is to write a
failing test, which Forge, Anvil and Kiln may never do. The control plane withholds it from
them; this repository cannot. §5 above and `vulcanflow-superpowers` §4 are what hold the
line instead. ADR-0007 §6.

---

## 7. Imports

The two files below are injected into every session at startup. `using-superpowers` is the
bootstrap that makes the rest trigger; `vulcanflow-superpowers` is the local adapter and
outranks it.

@.claude/skills/using-superpowers/SKILL.md
@.claude/skills/vulcanflow-superpowers/SKILL.md
