//! Illustrative package entitlements for tests.
//!
//! # These are not product values
//!
//! The architecture's open-question table records this explicitly: open
//! question **O9**, "package values and entitlement defaults", is owned by
//! **Product**, and its interim answer is "fixture entitlements in
//! `vf-testkit`, **labelled illustrative**". That label is this module.
//!
//! So: nothing here is a price, a quota, a plan name or a commitment. The
//! numbers exist so a test can exercise a code path that needs *some*
//! entitlement document — the graph validator refusing a node the package does
//! not include (§A2 `validate(graph, entitlements)`), the meter refusing a run
//! over an allowance — and they were chosen to make those paths easy to hit,
//! not to resemble what will be sold. A test that asserts a specific number
//! from this module is asserting on the fixture, and must say so in its name.
//!
//! When O9 is answered, the real defaults land wherever Product puts them and
//! this module keeps its own values. It does not become the source of truth by
//! being the only thing here first.
//!
//! # Why `serde_json::Value` and not `Entitlements`
//!
//! `vf_core::Entitlements` does not exist yet — §A2 declares
//! `validate(graph: &FlowGraph, entitlements: &Entitlements)` and task C1
//! writes the type. Until then a fixture is the JSON document, which is also
//! the shape §A3.7's `GET /v1/usage` returns and the shape the WASM validator
//! takes, so the fixtures are useful at both ends. The functions keep their
//! signatures and gain a typed sibling when C1 lands.

use crate::Result;

/// The illustrative entitlement document.
///
/// The one a test uses when it needs an entitlement document and does not care
/// which. Generous enough that nothing is refused for lack of entitlement, so
/// a refusal in a test using this fixture is about the thing under test.
pub fn fixture() -> Result<serde_json::Value> {
    Err(crate::Error::NotImplemented("entitlements::fixture"))
}

/// The illustrative entitlement document for `package`.
///
/// `package` is a `&str` rather than an enum because the package *names* are
/// the open question, not just the values: inventing an enum here would make
/// a placeholder look like a decision and would have to be deleted when
/// Product answers O9. [`Error::Fixture`][crate::Error::Fixture] names the
/// packages the corpus has when the lookup misses.
pub fn fixture_for(package: &str) -> Result<serde_json::Value> {
    let _ = package;
    Err(crate::Error::NotImplemented("entitlements::fixture_for"))
}

/// An entitlement document that includes nothing.
///
/// Every allowance zero and every optional capability absent, so a test can
/// prove the refusal path rather than only the allow path. §A7 invariant 1
/// ("no execution without basis, scope, reservation and evidence") needs this
/// as much as it needs [`fixture`].
pub fn empty() -> Result<serde_json::Value> {
    Err(crate::Error::NotImplemented("entitlements::empty"))
}

/// The package names the fixture corpus carries, sorted.
///
/// Illustrative, like everything else here. A test that iterates the corpus
/// uses this rather than hard-coding names, so answering O9 is a change to the
/// fixture tree and not to every test that mentions a package.
pub fn packages() -> Result<Vec<String>> {
    Err(crate::Error::NotImplemented("entitlements::packages"))
}
