//! Probe fixture for VUL-2. Not part of the workspace and not intended to compile.
//!
//! This file exists only so that one pull request can contain a production source
//! file and a test file at the same time, which is the thing `lane-partition` is
//! supposed to refuse. The pull request carrying it is expected to go red and to be
//! closed unmerged.

/// A coding agent (lane 3) would legitimately write this.
pub fn classify(n: i32) -> &'static str {
    if n < 0 { "negative" } else { "non-negative" }
}
