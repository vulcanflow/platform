//! T2 (VFL-41): `CanonicalHost` corpus — canonicalization and rejection rules.
//!
//! Written against architecture §A3.1 / TDD §5.3 (VFL-8 architecture document) ahead
//! of the C2 (VFL-15) API skeleton. `CanonicalHost::parse` is documented there as:
//! lowercase, strip one trailing dot, IDNA UTS-46 non-transitional to A-label, reject
//! IP literals, ports, paths, empty labels and public-suffix roots (via `psl`,
//! including private suffixes).
//!
//! ASSUMPTION pending C2 skeleton finalization: `ScopeType`/`ScopeVerdict`/`HostError`
//! derive at least `Debug`; `CanonicalHost` derives `Debug` and `PartialEq`/`Eq` (or
//! exposes equality through `as_str()`, which every comparison below uses so the pack
//! still compiles if full `PartialEq` is not derived). If the skeleton's `HostError`
//! variant names differ from "rejection with a reason", only the `is_err()` shape is
//! asserted here — no variant is matched by name.

use vf_core::scope::CanonicalHost;

fn parses_to(input: &str, expected: &str) {
    let host = CanonicalHost::parse(input)
        .unwrap_or_else(|e| panic!("expected {input:?} to parse, got {e:?}"));
    assert_eq!(host.as_str(), expected, "canonical form of {input:?}");
}

fn rejected(input: &str) {
    assert!(
        CanonicalHost::parse(input).is_err(),
        "expected {input:?} to be rejected"
    );
}

// --- lowercase, trailing dot ------------------------------------------------

#[test]
fn lowercases_mixed_case_host() {
    parses_to("EXAMPLE.com", "example.com");
    parses_to("WwW.Example.COM", "www.example.com");
}

#[test]
fn strips_single_trailing_dot() {
    parses_to("example.com.", "example.com");
    parses_to("EXAMPLE.COM.", "example.com");
}

#[test]
fn rejects_double_trailing_dot() {
    // A trailing dot is stripped once; a second one would leave an empty label.
    rejected("example.com..");
}

// --- IDNA UTS-46 non-transitional, xn-- round trip --------------------------

#[test]
fn idna_unicode_host_normalizes_to_a_label() {
    // Classic Punycode worked example (RFC 3492 / UTS-46): "münchen" -> "mnchen-3ya".
    parses_to("münchen.de", "xn--mnchen-3ya.de");
}

#[test]
fn xn_dash_dash_input_round_trips_unchanged() {
    parses_to("xn--mnchen-3ya.de", "xn--mnchen-3ya.de");
    parses_to("XN--MNCHEN-3YA.DE", "xn--mnchen-3ya.de");
}

#[test]
fn unicode_and_a_label_forms_of_same_host_are_equal() {
    let unicode = CanonicalHost::parse("münchen.de").expect("unicode host parses");
    let a_label = CanonicalHost::parse("xn--mnchen-3ya.de").expect("a-label host parses");
    assert_eq!(unicode.as_str(), a_label.as_str());
}

#[test]
fn rejects_malformed_xn_dash_dash_label() {
    // Not a valid Punycode payload for the xn-- prefix.
    rejected("xn--\u{2603}.com");
}

// --- structural rejections ---------------------------------------------------

#[test]
fn rejects_empty_input() {
    rejected("");
}

#[test]
fn rejects_empty_label() {
    rejected("a..b.com");
    rejected(".example.com");
}

#[test]
fn rejects_label_over_63_octets() {
    let label = "a".repeat(64);
    rejected(&format!("{label}.com"));
}

#[test]
fn accepts_label_at_63_octets() {
    let label = "a".repeat(63);
    let input = format!("{label}.com");
    let host = CanonicalHost::parse(&input).expect("63-octet label is valid");
    assert_eq!(host.as_str(), input.to_lowercase());
}

#[test]
fn rejects_host_over_253_octets() {
    // 4 labels of 63 'a's joined by dots, plus ".com" comfortably exceeds 253.
    let long = std::iter::repeat("a".repeat(63))
        .take(4)
        .collect::<Vec<_>>()
        .join(".");
    rejected(&format!("{long}.com"));
}

#[test]
fn rejects_ip_literal() {
    rejected("192.0.2.1");
    rejected("::1");
    rejected("[::1]");
    rejected("2001:db8::1");
}

#[test]
fn rejects_host_with_port() {
    rejected("example.com:8443");
}

#[test]
fn rejects_host_with_path() {
    rejected("example.com/admin");
}

#[test]
fn rejects_host_with_userinfo() {
    rejected("user@example.com");
}

#[test]
fn rejects_control_and_whitespace_characters() {
    rejected("exa\u{0}mple.com");
    rejected("exa mple.com");
    rejected("example.com\n");
}

// --- public-suffix root rejection (including private suffixes) --------------

#[test]
fn rejects_bare_icann_public_suffix() {
    rejected("com");
    rejected("co.uk");
}

#[test]
fn rejects_bare_private_suffix() {
    // github.io is on the PSL PRIVATE section; psl's default list includes it.
    rejected("github.io");
}

#[test]
fn accepts_host_under_private_suffix() {
    let host = CanonicalHost::parse("foo.github.io").expect("subdomain of a private suffix is a valid host");
    assert_eq!(host.as_str(), "foo.github.io");
}

// --- registrable_domain(), including private suffixes -----------------------

#[test]
fn registrable_domain_of_subdomain_is_the_icann_registrable_root() {
    let host = CanonicalHost::parse("www.example.com").expect("parses");
    let root = host.registrable_domain().expect("has a registrable domain");
    assert_eq!(root.as_str(), "example.com");
}

#[test]
fn registrable_domain_of_registrable_root_is_itself() {
    let host = CanonicalHost::parse("example.com").expect("parses");
    let root = host.registrable_domain().expect("has a registrable domain");
    assert_eq!(root.as_str(), "example.com");
}

#[test]
fn registrable_domain_under_private_suffix_keeps_the_suffix_label() {
    // Under the private PSL entry "github.io", the registrable unit is one label
    // above the suffix: "foo.github.io", not "github.io" itself.
    let host = CanonicalHost::parse("deep.foo.github.io").expect("parses");
    let root = host.registrable_domain().expect("has a registrable domain");
    assert_eq!(root.as_str(), "foo.github.io");
}

#[test]
fn registrable_domain_of_host_already_at_private_suffix_boundary_is_itself() {
    let host = CanonicalHost::parse("foo.github.io").expect("parses");
    let root = host.registrable_domain().expect("has a registrable domain");
    assert_eq!(root.as_str(), "foo.github.io");
}

// --- is_descendant_of(): dot-boundary only, never label-prefix ---------------

#[test]
fn subdomain_is_descendant_of_its_parent() {
    let child = CanonicalHost::parse("www.example.com").expect("parses");
    let root = CanonicalHost::parse("example.com").expect("parses");
    assert!(child.is_descendant_of(&root));
}

#[test]
fn deep_subdomain_is_descendant_of_a_distant_ancestor() {
    let child = CanonicalHost::parse("a.b.c.example.com").expect("parses");
    let root = CanonicalHost::parse("example.com").expect("parses");
    assert!(child.is_descendant_of(&root));
}

#[test]
fn label_prefix_lookalike_is_never_a_descendant() {
    // The acceptance-criteria example: "notexample.com" shares a string suffix with
    // "example.com" but is a completely different registrable domain.
    let lookalike = CanonicalHost::parse("notexample.com").expect("parses");
    let root = CanonicalHost::parse("example.com").expect("parses");
    assert!(!lookalike.is_descendant_of(&root));
}

#[test]
fn hyphen_prefix_lookalike_is_never_a_descendant() {
    let lookalike = CanonicalHost::parse("evil-example.com").expect("parses");
    let root = CanonicalHost::parse("example.com").expect("parses");
    assert!(!lookalike.is_descendant_of(&root));
}

#[test]
fn sibling_is_not_a_descendant() {
    let sibling = CanonicalHost::parse("other.com").expect("parses");
    let root = CanonicalHost::parse("example.com").expect("parses");
    assert!(!sibling.is_descendant_of(&root));
}

#[test]
fn unrelated_subdomain_structure_is_not_a_descendant() {
    // "example.com.evil.com" ends with "com" but is not under "example.com" at all.
    let candidate = CanonicalHost::parse("example.com.evil.com").expect("parses");
    let root = CanonicalHost::parse("example.com").expect("parses");
    assert!(!candidate.is_descendant_of(&root));
}
