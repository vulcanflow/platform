//! The fixture corpus under `crates/vf-testkit/fixtures/` (architecture §A1.3).
//!
//! That directory is a *neutral* lane path: the lane gate classifies
//! `crates/*/src/*` as production and `crates/*/tests/*` as test, and
//! `crates/vf-testkit/fixtures/*` as neither. That is deliberate — task S1
//! captures real scanner output and commits it here, and a coder adding a
//! sample does not thereby author a test.
//!
//! Names are relative paths inside that tree, with the extension, so
//! `fixtures::load("scanner-output/nuclei/cve-basic.json")` reads exactly the
//! file a reader can open. Resolution is confined to the tree: a name that
//! escapes it is [`crate::Error::FixtureEscape`], not a read.

use std::path::{Path, PathBuf};

use crate::Result;

/// Absolute path of the fixture tree for the compiled crate.
///
/// Resolved from `CARGO_MANIFEST_DIR` at compile time, so it is correct
/// whatever the working directory of the test binary is — which matters
/// because `cargo test` runs from the workspace root while `cargo test -p`
/// does not promise to.
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

/// Reads one fixture.
///
/// The bytes are read eagerly and held, because a fixture is small by
/// construction and a test that loads one usually looks at it more than once.
pub fn load(name: &str) -> Result<Fixture> {
    let _ = name;
    Err(crate::Error::NotImplemented("fixtures::load"))
}

/// Every fixture name under `prefix`, sorted, with `prefix` included in each
/// returned name.
///
/// Sorted so a test that iterates the corpus — the schema validation pack of
/// task S1, for instance — reports the same first failure on every machine.
pub fn list(prefix: &str) -> Result<Vec<String>> {
    let _ = prefix;
    Err(crate::Error::NotImplemented("fixtures::list"))
}

/// One loaded fixture, with its bytes.
#[derive(Debug, Clone)]
pub struct Fixture {
    name: String,
    path: PathBuf,
    bytes: Vec<u8>,
}

impl Fixture {
    /// The name this fixture was loaded under, relative to [`root`].
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The absolute path on disk, for an assertion message that a developer
    /// can paste into an editor.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The raw bytes. The corpus holds non-UTF-8 fixtures on purpose — a
    /// bounded parser has to be shown a malformed artifact — so bytes, not
    /// text, is the primitive.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The bytes as text, or [`crate::Error::Fixture`] when they are not
    /// UTF-8.
    pub fn text(&self) -> Result<&str> {
        Err(crate::Error::NotImplemented("Fixture::text"))
    }

    /// Parses the fixture as JSON into `T`.
    ///
    /// External schemas carry `deny_unknown_fields` (§A7 invariant 10), so
    /// this is also how a fixture that has drifted from its type is caught.
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        Err(crate::Error::NotImplemented("Fixture::json"))
    }

    /// Parses the fixture as untyped JSON, for a test that asserts on shape
    /// before the typed struct exists.
    pub fn value(&self) -> Result<serde_json::Value> {
        Err(crate::Error::NotImplemented("Fixture::value"))
    }
}
