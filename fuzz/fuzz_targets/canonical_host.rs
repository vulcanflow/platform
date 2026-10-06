//! VFL-145 (T2 addendum to C2 / VFL-15): fuzz target for `CanonicalHost`.
//!
//! The invariant under fuzz is totality, not acceptance (architecture §A3.1 /
//! TDD §5.3): every input is either `Ok(CanonicalHost)` or a typed
//! `HostError` — never a panic — and that must hold across the whole surface
//! named in VFL-145's scope, not just the `parse` constructor:
//! `CanonicalHost::{parse, as_str, registrable_domain, is_descendant_of}`.
//!
//! Two independent candidate strings are decoded per run so `is_descendant_of`
//! is exercised on a genuine pair rather than a value compared only against
//! itself; `arbitrary`'s derive on `&str` already skips a run whose bytes are
//! not valid UTF-8, via `libfuzzer-sys`'s `Arbitrary` support, so this target
//! does not need its own UTF-8 branch.
//!
//! Corpus seeding and crash triage are Test Runner's step (`cargo fuzz run
//! canonical_host`), per VFL-145 and VFL-142 (nightly toolchain availability).

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use vf_core::scope::CanonicalHost;

/// Two independent candidate hostnames, decoded from one fuzz input.
#[derive(Debug, Arbitrary)]
struct HostPair<'a> {
    a: &'a str,
    b: &'a str,
}

fuzz_target!(|pair: HostPair| {
    let a = CanonicalHost::parse(pair.a);
    let b = CanonicalHost::parse(pair.b);

    if let Ok(host) = &a {
        let _ = host.as_str();
        let _ = host.registrable_domain();
    }
    if let Ok(host) = &b {
        let _ = host.as_str();
        let _ = host.registrable_domain();
    }
    if let (Ok(x), Ok(y)) = (&a, &b) {
        let _ = x.is_descendant_of(y);
        let _ = y.is_descendant_of(x);
    }
});
