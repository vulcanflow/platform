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
//!
//! # What counts as a fixture
//!
//! A regular file under the tree, other than a `README.md` and other than a
//! file whose name starts with `.`. The README rule is the corpus's own: a
//! fixture that needs explaining gets a `README.md` beside it, and a test that
//! iterates a directory with [`list`] must not be handed the explanation as if
//! it were input.
//!
//! # Confinement
//!
//! Two checks, because either alone has a hole. The name is checked as
//! written — plain relative segments only, no `..`, no root — and then the
//! path it resolves to is canonicalised and must still be inside the
//! canonical tree, which is what catches a symbolic link somewhere along the
//! way. A link as the fixture itself is refused outright, and [`list`] does
//! not descend into or report links, so the corpus a test iterates is the
//! corpus a reader sees in git.

use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::{Error, Result};

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
    if name.is_empty() {
        return Err(Error::Fixture {
            name: String::new(),
            message: "a fixture name cannot be empty".to_owned(),
        });
    }
    let path = resolve(name)?;

    // `symlink_metadata`, not `metadata`, so a link as the fixture itself is
    // seen as a link and refused rather than followed.
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::FixtureNotFound {
                name: name.to_owned(),
                path,
            });
        }
        Err(source) => return Err(Error::Io { path, source }),
    };
    if meta.file_type().is_symlink() {
        return Err(Error::FixtureEscape(name.to_owned()));
    }
    if !meta.is_file() {
        return Err(Error::Fixture {
            name: name.to_owned(),
            message: format!("{} is not a regular file", path.display()),
        });
    }
    ensure_inside(&path, name)?;

    let bytes = fs::read(&path).map_err(|source| Error::Io {
        path: path.clone(),
        source,
    })?;
    Ok(Fixture {
        name: name.to_owned(),
        path,
        bytes,
    })
}

/// Every fixture name under `prefix`, sorted, with `prefix` included in each
/// returned name.
///
/// Sorted so a test that iterates the corpus — the schema validation pack of
/// task S1, for instance — reports the same first failure on every machine.
///
/// `prefix` is a directory inside the tree, with or without a trailing `/`;
/// the empty prefix is the whole tree. A prefix that does not exist is
/// [`Error::FixtureNotFound`] rather than an empty list, because a test that
/// iterates a misspelt directory would otherwise pass by checking nothing.
/// Names use `/` as the separator on every platform, so they can be passed
/// straight back to [`load`].
pub fn list(prefix: &str) -> Result<Vec<String>> {
    // Checked before the trailing `/` is trimmed, which would otherwise turn
    // `/` into the empty prefix and so into the whole tree.
    if prefix.starts_with('/') {
        return Err(Error::FixtureEscape(prefix.to_owned()));
    }
    let prefix = prefix.trim_end_matches('/');
    let dir = resolve(prefix)?;

    match fs::symlink_metadata(&dir) {
        Ok(meta) if meta.is_dir() => {}
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(Error::FixtureEscape(prefix.to_owned()));
        }
        Ok(_) => {
            return Err(Error::Fixture {
                name: prefix.to_owned(),
                message: format!("{} is not a directory", dir.display()),
            });
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::FixtureNotFound {
                name: prefix.to_owned(),
                path: dir,
            });
        }
        Err(source) => return Err(Error::Io { path: dir, source }),
    }
    ensure_inside(&dir, prefix)?;

    let mut names = Vec::new();
    walk(&dir, prefix, &mut names)?;
    names.sort();
    Ok(names)
}

/// Joins `name` onto [`root`], refusing anything but plain relative segments.
///
/// The lexical half of the confinement in the module header. `.` segments
/// are refused too — not because they escape, but because a name is meant to
/// be the path a reader can open, and `./a` and `a` being one fixture under
/// two names helps nobody. Empty segments (`a//b`, a trailing `/`) are
/// refused for the same reason.
///
/// The segments are checked as written, split on `/`, because
/// `Path::components` normalises `a/./b` and `a//b` to `a/b` and so cannot
/// see either. The empty name is the tree itself: [`list`] passes it for the
/// whole corpus, and [`load`] refuses it before it gets here.
fn resolve(name: &str) -> Result<PathBuf> {
    let as_written = name.is_empty()
        || name
            .split('/')
            .all(|segment| !matches!(segment, "" | "." | ".."));
    let relative = Path::new(name);
    let normal = relative
        .components()
        .all(|c| matches!(c, Component::Normal(_)));
    if !(as_written && normal) {
        return Err(Error::FixtureEscape(name.to_owned()));
    }
    Ok(root().join(relative))
}

/// The canonical half of the confinement: `path`, with every link along it
/// resolved, must still be inside the tree.
fn ensure_inside(path: &Path, name: &str) -> Result<()> {
    let canonical = |p: &Path| {
        fs::canonicalize(p).map_err(|source| Error::Io {
            path: p.to_owned(),
            source,
        })
    };
    if canonical(path)?.starts_with(canonical(&root())?) {
        Ok(())
    } else {
        Err(Error::FixtureEscape(name.to_owned()))
    }
}

/// Collects the fixture names under `dir`, which is `prefix` inside the tree.
fn walk(dir: &Path, prefix: &str, names: &mut Vec<String>) -> Result<()> {
    let entries = fs::read_dir(dir).map_err(|source| Error::Io {
        path: dir.to_owned(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| Error::Io {
            path: dir.to_owned(),
            source,
        })?;
        let path = entry.path();
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            return Err(Error::Fixture {
                name: path.display().to_string(),
                message: "a fixture name must be UTF-8".to_owned(),
            });
        };
        if file_name.starts_with('.') || file_name == "README.md" {
            continue;
        }
        let name = if prefix.is_empty() {
            file_name.to_owned()
        } else {
            format!("{prefix}/{file_name}")
        };

        // `DirEntry::file_type` does not follow links, so a link is neither a
        // file nor a directory here and is skipped.
        let file_type = entry.file_type().map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        if file_type.is_dir() {
            walk(&path, &name, names)?;
        } else if file_type.is_file() {
            names.push(name);
        }
    }
    Ok(())
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
        std::str::from_utf8(&self.bytes).map_err(|e| self.error(format!("not UTF-8: {e}")))
    }

    /// Parses the fixture as JSON into `T`.
    ///
    /// External schemas carry `deny_unknown_fields` (§A7 invariant 10), so
    /// this is also how a fixture that has drifted from its type is caught.
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_slice(&self.bytes)
            .map_err(|e| self.error(format!("not a valid `{}`: {e}", std::any::type_name::<T>())))
    }

    /// Parses the fixture as untyped JSON, for a test that asserts on shape
    /// before the typed struct exists.
    pub fn value(&self) -> Result<serde_json::Value> {
        serde_json::from_slice(&self.bytes).map_err(|e| self.error(format!("not JSON: {e}")))
    }

    /// [`Error::Fixture`] for this fixture.
    fn error(&self, message: String) -> Error {
        Error::Fixture {
            name: self.name.clone(),
            message,
        }
    }
}
