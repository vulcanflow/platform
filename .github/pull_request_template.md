<!--
  VulcanFlow pull-request template.

  Every section below is required. The lane 5.5 block at the bottom is checked
  mechanically by `pre-pr-review-verdict`; the rest is checked by Assay in lane 6.

  Authority: ADR-0005 (`vulcanflow/docs` — decisions/ADR-0005-delivery-pipeline-and-lane-enforcement.md)
  and `process/agent-workflow.md`. Where this template and ADR-0005 disagree, ADR-0005 wins and
  this file is the thing that gets corrected.
-->

## What this changes

<!-- One paragraph. What moved, and why it had to. -->

## Lane

<!--
  Name your lane and the issue. One change, one lane: a diff carrying both production
  source and a test file is refused by `lane-partition` before anyone reads it.
-->

- **Lane:** <!-- 1 spec | 2 tests | 3 code | 5 red→fix | NEUTRAL (docs, process, ci) -->
- **Issue:** VUL-
- **§25 identifiers in scope:** <!-- e.g. `api/dispatcher-outbox-ordering`, or: n/a (no §25 identifier in scope) -->
- **Acceptance statement from the lane-1 spec:** <!-- the one sentence a stranger could judge this against -->

## Pre-PR CodeRabbit CLI review (lane 5.5)

<!--
  Lane 5.5 is the change author's own pre-flight, run BEFORE this pull request existed,
  and re-run before every later push that changes the diff. It is not a review: it
  produces no verdict and is zero of lane 6's two required verdicts (ADR-0005 §6.1, §6.2
  corollary 1). It gates entry to lane 6, not the merge.

  Run it as described in the `vulcanflow-pre-pr-review` company skill, which holds the
  CLI's absolute path. Export it once as CODERABBIT_BIN; do not paste an instance path
  into this public repository.

      "$CODERABBIT_BIN" review --agent --base main

  Fill every row from the run you actually did. `Tree reviewed` is the sha this pull
  request's head is at — a block naming a superseded commit is a stale block, and a push
  that changes the diff needs a fresh run and a fresh block.
-->

| | |
|---|---|
| CLI version | <!-- e.g. 0.8.2 --> |
| Command | `$CODERABBIT_BIN review --agent --base main` |
| Tree reviewed | <!-- the sha this PR's head is at --> |
| Findings | 0 |
| Run at | <!-- UTC, e.g. 2026-10-02T09:40Z --> |

Declined findings: none.

<!--
  `Findings` must read 0. The board's rule of 2026-10-01 19:49Z is "all issues fixed"
  before the pull request exists, so the only value that passes is zero; whether a
  severity floor with a declined-findings path ever replaces that is an open board
  question (VUL-1), and until it is answered `Declined findings: none.` is the only
  admissible value of that line.

  Batch your fixes. One push per fix round, not one push per comment — fix everything
  the round raised, re-run the CLI to zero, then push once. The CodeRabbit App re-reviews
  on every push and its allowance is finite.
-->

## Merge precondition

Merge is **Crucible's**, on the conditions in **ADR-0005 §6.3** — read that section; it is
the only statement of them and this line is a pointer, not a restatement. Two things about
this pull request that §6.3 needs and only the author can supply:

- **Two lane-6 verdicts** — Assay by hand, Warren on the CodeRabbit App's review — each
  recorded on that reviewer's own Paperclip issue, each stating `APPROVE` or
  `REQUEST CHANGES`, each naming the head sha it covers. Any push invalidates both.
- **A clean CodeRabbit App review at this pull request's head.** A review on a superseded
  commit does not carry forward.

<!--
  A lane 5.5 block is not an approval and never counts toward the pair.
  Crucible: do not merge on a count of artefacts present. Check each condition.
-->

## Checklist

- [ ] One lane. No diff mixing production source with test files, or either with the gate.
- [ ] Lane 5.5 run on this pull request's **current** head, zero findings, block above filled in.
- [ ] No credential, token or `.env` in the diff or in this description.
- [ ] Dependency pins unchanged, or changed with the ADR that records why (ADR-0002).
- [ ] Reconciled forward onto the base. Never force-pushed.
