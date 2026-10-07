# Illustrative entitlements

**Everything in this directory is illustrative.** None of it is a price, a
plan, a package name or a commitment. Architecture open question **O9**
(package values and entitlement defaults) belongs to Product and is
unanswered; its recorded interim answer is "fixture entitlements in
`vf-testkit`, labelled illustrative", and this is that label.

Read through [`vf_testkit::entitlements`](../../src/entitlements.rs):

| File | Function | Meant to exercise |
|---|---|---|
| `packages/fixture-full.json` | `fixture()`, `fixture_for("fixture-full")` | the allow path: every node type, ceilings no test reaches by accident |
| `packages/fixture-basic.json` | `fixture_for("fixture-basic")` | a partial package: discovery and HTTP nodes only, so a graph with `nuclei`, `nmap` or `masscan` is refused as not entitled, and low ceilings a test can exceed on purpose |
| `empty.json` | `empty()` | the refusal path: no node type and every ceiling zero |

`packages/` holds one document per illustrative package and nothing else,
because `entitlements::packages()` lists it. The names start with `fixture-`
so that no one mistakes them for the names Product will choose.

## Shape

Each document is the `Entitlements` struct of architecture §A2 (task G1 writes
it in `vf-graph`), serialised as JSON, with exactly its three fields:

```json
{ "nodes": ["subfinder", "dnsx"], "max_hosts_per_run": 10, "max_scanner_minutes_per_run": 30 }
```

Node types are the §A2 `NodeType` variants in lower case. That casing is an
assumption until G1 fixes the serde representation of `NodeType`; if G1 picks
another, these files change with it. No other key is present, not even a
comment or an "illustrative" marker, because the real struct is expected to
deny unknown fields and a fixture that cannot be deserialised into it is no
use to the validator tests.

The numbers were chosen to make the allow and refuse paths easy to reach. A
test that asserts on one of them is asserting on this fixture, and says so in
its name.
