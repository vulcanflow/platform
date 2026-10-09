//! VFL-145 (T2 addendum to C2 / VFL-15): fuzz target for `CanonicalHost`.
//!
//! The invariant under fuzz is totality, not acceptance (architecture §A3.1 /
//! TDD §5.3): every input is either `Ok(CanonicalHost)` or a typed
//! `HostError` — never a panic — and that must hold across the whole surface
//! named in VFL-145's scope, not just the `parse` constructor:
//! `CanonicalHost::{parse, as_str, registrable_domain, is_descendant_of}`.
//!
//! Rework per [VFL-199 F2/F4](/VFL/issues/VFL-199#document-review-c39241c):
//! the prior version decoded two independent `&str`s straight from the fuzz
//! buffer. `&str::arbitrary` draws its length from the tail of the remaining
//! buffer and truncates at the first invalid UTF-8 byte (`arbitrary` 1.4.2,
//! `arbitrary_str`), so `parse`'s gates are rarely cleared and the two
//! strings almost never share a suffix — `is_descendant_of` was exercised on
//! an unrelated pair, and IPv4-shaped, numeric-last-label, punycode-shaped
//! and the 63/253-byte boundary inputs were reached only by mutation luck.
//! (That earlier doc comment's claim that non-UTF-8 input is skipped rather
//! than truncated was itself wrong — F4 — and is corrected here: `parse`
//! takes `&str`, so nothing is lost by the domain being UTF-8 prefixes
//! rather than literal byte pairs; it just means the degenerate-input claim
//! below cannot rely on raw bytes to reach those shapes and needs its own
//! arms instead.)
//!
//! [`HostSeed`] now has a named arm per boundary category named in VFL-145's
//! focus areas, plus a `Raw` escape hatch that still exercises the full
//! `&str` domain (and so still proves totality over inputs no arm
//! constructs). `related` derives the second host from the first with
//! meaningful probability, so `is_descendant_of` sees genuine ancestor/
//! descendant and sibling pairs rather than two coincidentally-unrelated
//! strings.
//!
//! Corpus seeding and crash triage are Test Runner's step (`cargo fuzz run
//! canonical_host`, documented in `fuzz/README.md`), per VFL-145 and
//! VFL-201 (nightly + cargo-fuzz toolchain provisioning).

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use vf_core::scope::CanonicalHost;

/// Known-good registrable-domain bases: each is suffix-plus-one under the
/// Public Suffix List, so a label stack built on one of these reliably
/// clears `parse`'s suffix-root and syntax gates and reaches the IDNA /
/// registrable-domain / descendant surface the earlier, unstructured
/// generation rarely did (VFL-199 F2).
const BASES: [&str; 3] = ["example.com", "example.org", "example.co.uk"];

/// A small label alphabet, all lowercase ASCII so every draw clears the
/// post-IDNA `[a-z0-9-]` gate without depending on IDNA folding to get there.
const LABELS: [&str; 6] = ["a", "b", "api", "sub", "stage", "www"];

/// One host, generated either from a boundary-category arm or as raw bytes.
#[derive(Debug, Arbitrary)]
enum HostSeed<'a> {
    /// `label.label...base`, 0..=4 labels deep over a known-good base.
    Labels(Vec<u8>, u8),
    /// An IPv4-literal-shaped string (`a.b.c.d`) — `parse` must reject this
    /// as a typed error, not panic on it.
    Ipv4Shaped(u8, u8, u8, u8),
    /// A host whose last label is all digits (`foo.1`): a distinct rejection
    /// path from the IPv4-literal one (not four dot-separated octets).
    NumericLastLabel(Vec<u8>, u16),
    /// A punycode-shaped label (`xn--...`), to exercise the IDNA decode path
    /// rather than the plain-ASCII one.
    Punycode(Vec<u8>),
    /// Straddles the 63-byte single-label boundary from both sides.
    LongLabel(u8),
    /// Straddles the 253-byte whole-name boundary from both sides.
    LongName(u8),
    /// Escape hatch: raw bytes, unconstrained — keeps the totality claim
    /// honest over inputs no structured arm above would construct.
    Raw(&'a str),
}

/// How the second host in a pair relates to the first, so `is_descendant_of`
/// is exercised on genuine ancestor/descendant pairs with meaningful
/// probability instead of almost never (VFL-199 F2).
#[derive(Debug, Arbitrary)]
enum Related {
    /// Unrelated: `b` is its own independent [`HostSeed`].
    Independent,
    /// Identical string.
    Same,
    /// `b` is `a` with 1..=3 extra labels prepended — a genuine descendant.
    Descendant(Vec<u8>),
}

#[derive(Debug, Arbitrary)]
struct HostPair<'a> {
    a: HostSeed<'a>,
    b_seed: HostSeed<'a>,
    related: Related,
}

fn label_stack(indices: &[u8], max_depth: usize) -> Vec<&'static str> {
    indices
        .iter()
        .take(max_depth)
        .map(|i| LABELS[usize::from(*i) % LABELS.len()])
        .collect()
}

fn materialize(seed: &HostSeed) -> String {
    match seed {
        HostSeed::Labels(indices, base) => {
            let base = BASES[usize::from(*base) % BASES.len()];
            let mut parts = label_stack(indices, 4);
            parts.push(base);
            parts.join(".")
        }
        HostSeed::Ipv4Shaped(a, b, c, d) => format!("{a}.{b}.{c}.{d}"),
        HostSeed::NumericLastLabel(indices, n) => {
            let mut parts: Vec<String> = label_stack(indices, 3)
                .into_iter()
                .map(ToOwned::to_owned)
                .collect();
            parts.push(n.to_string());
            parts.join(".")
        }
        HostSeed::Punycode(indices) => {
            let digits: String = indices
                .iter()
                .take(8)
                .map(|i| char::from(b'a' + (*i % 26)))
                .collect();
            format!("xn--{digits}.com")
        }
        HostSeed::LongLabel(n) => {
            // 63 is the single-label boundary (scope.rs's DNS label check);
            // +/- 9 straddles it from both sides.
            let len = 59 + usize::from(*n % 10);
            format!("{}.com", "a".repeat(len))
        }
        HostSeed::LongName(n) => {
            // 253 is the whole-name boundary; repeated 2-byte labels ("a.")
            // straddle it from both sides without any single label being
            // long enough to trip the 63-byte check instead.
            let labels = 121 + usize::from(*n % 10);
            let mut s = "a.".repeat(labels);
            s.push_str("com");
            s
        }
        HostSeed::Raw(s) => (*s).to_owned(),
    }
}

fuzz_target!(|pair: HostPair| {
    let a_str = materialize(&pair.a);
    let b_str = match &pair.related {
        Related::Independent => materialize(&pair.b_seed),
        Related::Same => a_str.clone(),
        Related::Descendant(indices) => {
            let mut parts = label_stack(indices, 3);
            parts.push(a_str.as_str());
            parts.join(".")
        }
    };

    let a = CanonicalHost::parse(&a_str);
    let b = CanonicalHost::parse(&b_str);

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
