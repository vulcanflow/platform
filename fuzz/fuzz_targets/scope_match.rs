//! T2 (VFL-41) fuzz target for C2's `check_candidate` / `check_run_scope`
//! (architecture §A3.1, TDD §25 `authz/configured-scope`). Only `CanonicalHost`
//! values that already passed `CanonicalHost::parse` are fed into scope
//! matching, since that is the only way production code can obtain one; the
//! property fuzzed is "never panics", matching `canonical_host.rs`.
//!
//! Input layout: byte 0's bit 0 selects `ScopeType`, bit 1 selects
//! `include_subdomains`; the rest of the input is UTF-8 text split on a NUL
//! byte into a root-host candidate and a target-host candidate. Either half
//! failing `CanonicalHost::parse` just ends the run early rather than padding
//! in a fixed fallback host, so every executed case is two real parsed hosts.

#![no_main]

use libfuzzer_sys::fuzz_target;
use vf_core::scope::{check_candidate, ApprovedScope, CanonicalHost, RunScope, ScopeType};

fuzz_target!(|data: &[u8]| {
    let Some((&flags, rest)) = data.split_first() else {
        return;
    };
    let scope_type = if flags & 1 == 0 {
        ScopeType::ExactHost
    } else {
        ScopeType::DomainTree
    };
    let include_subdomains = flags & 2 != 0;

    let Ok(text) = std::str::from_utf8(rest) else {
        return;
    };
    let mut parts = text.splitn(2, '\u{0}');
    let (Some(root_str), Some(candidate_str)) = (parts.next(), parts.next()) else {
        return;
    };

    // Parsed twice rather than cloned: the architecture contract (§A3.1) does not
    // guarantee `CanonicalHost: Clone`, only `parse`/`as_str`/`registrable_domain`/
    // `is_descendant_of`, and this target should not add assumptions beyond that.
    let (Ok(approval_root), Ok(run_target)) =
        (CanonicalHost::parse(root_str), CanonicalHost::parse(root_str))
    else {
        return;
    };
    let Ok(candidate) = CanonicalHost::parse(candidate_str) else {
        return;
    };

    let approval = ApprovedScope {
        scope_type,
        root: approval_root,
    };
    let run = RunScope {
        target: run_target,
        include_subdomains,
    };
    let _ = check_candidate(&approval, &run, &candidate);
});
