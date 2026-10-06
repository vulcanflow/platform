//! T2 (VFL-41) fuzz target for C2's `CanonicalHost::parse` (architecture §A3.1).
//!
//! `parse` takes arbitrary attacker-controlled host text (run submission, hook
//! payloads, discovered-host output), so the only property fuzzed here is "never
//! panics, for any byte string" — libFuzzer treats an unwind as a crash on its
//! own; this target just has to drive the function without adding assumptions
//! of its own (no `unwrap`, no slicing past validated bounds).

#![no_main]

use libfuzzer_sys::fuzz_target;
use vf_core::scope::CanonicalHost;

fuzz_target!(|data: &[u8]| {
    let Ok(input) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(host) = CanonicalHost::parse(input) {
        let _ = host.as_str();
        let _ = host.registrable_domain();
        let _ = host.is_descendant_of(&host);
    }
});
