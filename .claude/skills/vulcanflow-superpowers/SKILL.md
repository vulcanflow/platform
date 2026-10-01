---
name: vulcanflow-superpowers
description: How the Superpowers skill library is used at vulcanFlow, and the exact points where a Superpowers workflow collides with the seven-lane delivery pipeline (ADR-0005) and must yield. Read this before invoking any Superpowers skill. Covers the mandatory skill-invocation rule, the per-lane skill map, and the subagent-inheritance rule.
---

# Superpowers at vulcanFlow

The [Superpowers](https://github.com/obra/superpowers) library (15 skills, pinned to
upstream commit `8ca22dba`) is installed in this company's skill catalog. Those skills
are not optional reading — they are how work is done here.

This skill is the local adapter. Superpowers was written for a single agent that
specs, tests, codes, runs and merges its own work. vulcanFlow splits those into seven
lanes held by different agents. Where a Superpowers skill assumes you hold the next
lane, it is wrong about vulcanFlow and this document says so explicitly.

## 1. The rule (non-negotiable)

**Before any response or action — including clarifying questions, reading the
codebase, or checking a file — invoke the skills that apply.** If you think there is
even a 1% chance a skill applies, invoke it. Announce `Using [skill] to [purpose]`
and follow it. If it has a checklist, make a todo per item.

Process skills come first and set the approach; implementation skills carry it out.

- "Build X" / "add Y" / "change behaviour" → `brainstorming` first.
- **Anything that produces a plan → `brainstorming` first, always.** A plan document,
  an implementation plan, a roadmap, a task breakdown, entering plan mode: none of them
  start until `brainstorming` has run and its design is confirmed. "Write me a plan" is
  not permission to skip to `writing-plans`. Only an explicit in-the-moment "skip
  brainstorming" from the board overrides this. (Board instruction, 2026-10-01.)
- "Fix this bug" / a failing test / unexpected behaviour → `systematic-debugging` first.
- "Here is a spec, implement it" → `writing-plans`, then `executing-plans`.
- About to say *done*, *fixed*, *passing*, or open a PR → `verification-before-completion`.

If it turns out wrong for the situation, you do not have to use it — but you must
have looked. "This is simple", "I need context first", "the skill is overkill" and
"I remember this skill" are all rationalisations. Skills evolve; read the current one.

## 2. Precedence

Your `AGENTS.md`, the TDD, and the ADR series outrank every Superpowers skill.
Superpowers itself says so: *user instructions take precedence over skills*. A
Superpowers skill never authorises you to cross a lane boundary.

Order, highest first:

1. ADR series and TDD v2.3 (design of record)
2. Your `AGENTS.md` — your lane and its prohibitions
3. This skill — the vulcanFlow reading of a Superpowers workflow
4. The Superpowers skill text itself

## 3. The subagent-inheritance rule

**A subagent you dispatch is you.** It inherits your lane and every prohibition in
your `AGENTS.md`.

`dispatching-parallel-agents`, `subagent-driven-development` and
`executing-plans` all encourage farming work out to subagents. None of them knows
about lanes. So, concretely:

- A coding agent (Forge, Anvil, Kiln) may **not** dispatch a subagent to write,
  edit, delete, rename, skip or `#[ignore]` a test. Delegating a prohibited act does
  not launder it.
- A test author (Scribe, Ledger) may **not** dispatch a subagent to run a suite or to
  read production source.
- Nobody but Crucible may dispatch a subagent that runs a suite or merges.

If a plan step needs a lane you do not hold, stop and hand off to the agent who holds
it. That is a comment on the issue, not a subagent.

## 4. Where Superpowers collides with the pipeline

### `test-driven-development`

Upstream tells one agent to write a failing test, watch it fail, then make it pass.
At vulcanFlow that loop is split across three agents and nobody gets to run it.

| You are | What the skill means here |
|---|---|
| Atlas | The discipline is real but you express it as a §25 identifier plus an acceptance statement. You write neither the test nor the code. |
| Scribe, Ledger | You write the test from the spec and stop. You never run it, so you never see red. Red-by-construction is the deliverable; Crucible observes the colour. |
| Forge, Anvil, Kiln | The test already exists and is already red. You start at step 3. You do not write the failing test — writing it would violate your lane. |
| Assay | Use it as a review lens: does the test actually pin the behaviour the identifier names, or does it pass vacuously? |
| Crucible | You are the only agent who ever observes red or green. |

### `verification-before-completion`

Upstream: run the verification command and read its output before claiming anything
works. Correct, and most agents here cannot run it.

- Forge, Anvil, Kiln, Scribe, Ledger: your evidence is **Crucible's PASS/FAIL/MISSING
  ledger**, quoted by §25 identifier. "It builds locally" is not evidence that a
  suite passes, and running the suite yourself to get evidence is a lane violation.
  The skill's ban on unverified success claims still binds you completely: without a
  ledger line, you do not say *passing*.
- Crucible: the skill applies literally. Run it, read the output, publish the ledger.
- Assay, Warren: verify that the claim in the PR body matches a ledger line.

### `requesting-code-review` and `receiving-code-review`

Upstream's reviewer subagent is a **preparation and absorption** tool, not the gate.
The gate is lane 6: Assay by hand **and** Warren via CodeRabbit, both approving.
Running `requesting-code-review` on your own work does not produce an approval and
never counts toward the two. Use it to catch what Assay would catch before Assay
has to. `receiving-code-review` applies as written — verify a finding before
implementing it, and do not perform agreement with something you think is wrong.

### `finishing-a-development-branch`

Only Crucible merges, only on a green ledger plus two approvals. Every other agent
stops at "push the branch, open the PR, hand off". Nobody else evaluates integration
options.

### `brainstorming`

Use it for genuinely open design questions. It is **not** a licence to reopen a
decided ADR. Rust (ADR-0001), the crate set and pins (ADR-0002), the Phase 0 gates
(ADR-0003), the workspace layout (ADR-0004) and the pipeline (ADR-0005) are decided.
If you believe one is wrong, that is an ADR amendment via Atlas, not a brainstorm.

### `writing-skills`

Atlas and CEO only. A new skill is process, and process is Atlas's lane.

### `using-git-worktrees`

Applies as written. Isolate before you start. Note that the one shared GitHub
identity means a worktree does not give you a separate reviewer — see VUL-9.

## 5. Which skills you hold

| Skill | Who |
|---|---|
| `using-superpowers`, `brainstorming`, `systematic-debugging`, `verification-before-completion`, `writing-plans`, `executing-plans`, `requesting-code-review`, `receiving-code-review`, `using-git-worktrees`, `vulcanflow-superpowers` | everyone |
| `test-driven-development` | Atlas, Scribe, Ledger, Assay |
| `finishing-a-development-branch` | Crucible, Atlas |
| `dispatching-parallel-agents`, `subagent-driven-development`, `writing-skills`, `diagnosing-superpowers` | CEO, Atlas |

`test-driven-development` is deliberately withheld from Forge, Anvil and Kiln: its
first instruction is to write a failing test, which is the one thing they may never
do. They get the discipline through the spec and the pre-existing red test instead.

## 6. Helper scripts

Five Superpowers skills ship executable helpers (`brainstorming`, `executing-plans`,
`subagent-driven-development`, `systematic-debugging`, `writing-skills`). Paperclip
does not install externally sourced executables into an agent runtime, so those files
are absent. Each affected skill carries a note saying which. Where a step names a
missing script, do the step by hand. No workflow depends on one.

## 7. If this skill is wrong

If a Superpowers skill tells you to do something your `AGENTS.md` forbids and this
document does not cover the case, do not resolve it yourself and do not quietly pick
the lane-violating branch. Say so in a comment on the issue and route it to Atlas,
who owns ADR-0005. Then continue with the part of the work that is unambiguous.
