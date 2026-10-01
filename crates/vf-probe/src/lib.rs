//! Probe crate for VUL-2. Production source, deliberately bundled with a test
//! file in the same pull request so that branch protection can be observed
//! refusing the merge. Not part of the workspace; delete with the probe branch.

/// Returns the lane a path belongs to, in the crudest possible form.
pub fn lane_of(path: &str) -> &'static str {
    if path.contains("/tests/") {
        "test"
    } else {
        "production"
    }
}
