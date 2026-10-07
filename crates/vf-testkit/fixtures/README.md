# `vf-testkit` fixtures

Input data for tests, loaded through [`vf_testkit::fixtures`](../src/fixtures.rs):

```rust
let f = vf_testkit::fixtures::load("scanner-output/nuclei/cve-basic.json")?;
let doc: NucleiFindings = f.json()?;
```

A fixture name is its path under this directory, extension included, so the
name in a test is the file a reader can open.

## This directory is in the neutral lane

The lane gate (`ci/lane-gate.sh`) classifies `crates/*/src/*` as production and
`crates/*/tests/*` as test, and refuses a pull request that changes both. This
directory is neither, by design (architecture §A1.3): task S1 captures real
scanner output and commits it here, and **adding a sample is not authoring a
test**, so a coder may do it without crossing a lane.

The corollary is that a file here must be *data*. An assertion, an expected
result or a golden output is a test, and belongs in `crates/<crate>/tests/`,
`conformance/` or a `*.golden` file — all of which the gate classifies as test.
If you cannot describe a new file as "something the system might be given",
it does not go here.

## Layout

One directory per source of data. A directory is created by the task that
needs it; this list grows and nothing is reserved in advance.

| Directory | Holds | Added by |
|---|---|---|
| `scanner-output/<tool>/` | raw output of a scanner, exactly as the tool wrote it | S1 |
| `graph/` | flow-graph documents for the validator | G1 |
| `entitlements/` | illustrative package entitlements — see below | F2 / O9 |
| `notifications/` | `vf-hook-notify` completion payloads, signed and unsigned | F2 / I1 |
| `dns/` | zone data for the local authoritative server behind `ChallengeProbe` | G3 |

## Rules for a fixture

1. **Verbatim.** Scanner output is committed as the tool produced it, not
   reformatted, not pretty-printed, not with ids replaced. A parser that is
   only ever shown tidied input is not tested (§A7 invariant 6: ingest is
   bounded, and malformed input is a platform outcome).
2. **Small.** Keep a fixture under about 64 KiB. When the point of a fixture is
   size — an oversize artifact that must be refused — generate it in the test
   from a committed small one rather than committing megabytes.
3. **Malformed on purpose is welcome, and named so.** Truncated JSON, a NUL
   byte, a duplicate finding id, a bad UTF-8 sequence: all are real inputs. Put
   `-malformed`, `-truncated` or `-duplicate` in the file name so a reader of
   the test knows the input is deliberate. `Fixture::bytes` is the primitive
   precisely so these can exist; `Fixture::text` is the one that fails on them.
4. **No real-world identifiers.** No customer name, no real hostname you do not
   control, no credential, no token, no IP outside the ranges reserved for
   documentation (RFC 5737, RFC 3849). Hostnames use `.test`, `.example` or
   `example.com`. This directory is public and permanent.
5. **A fixture that needs explaining gets a `README.md` beside it**, in its own
   directory, saying where the data came from, which tool version produced it
   and what it is meant to exercise.

## `entitlements/`

Everything in `entitlements/` is **illustrative** and is not a price, a plan or
a commitment. Architecture open question **O9** — package values and
entitlement defaults — is owned by Product and is unanswered; the interim
answer recorded there is exactly "fixture entitlements in `vf-testkit`,
labelled illustrative". The values here were chosen to make refusal and
allowance paths easy to reach, not to resemble anything that will be sold. See
[`vf_testkit::entitlements`](../src/entitlements.rs).
