//! Canonical hosts, scope matching and IPv4 destinations — architecture
//! §A3.1, TDD §5.3 and §5.6. Task C2.
//!
//! This module is the single place where an untrusted hostname becomes a value
//! the rest of the platform is allowed to act on. Threat model rows "subdomain
//! approval expands" and "annotation differs from actual target" both land
//! here, so the module fails closed: every constructor rejects by default and
//! every rejection carries a named reason ([`HostError`]).
//!
//! Three invariants hold for every value this module hands out:
//!
//! 1. A [`CanonicalHost`] is a DNS domain name in A-label form, lowercased,
//!    with no trailing dot, no port, no path, no userinfo and no IP literal,
//!    and it is never a public-suffix root (ICANN or private section).
//! 2. [`CanonicalHost::is_descendant_of`] matches on a dot boundary only, so a
//!    label prefix such as `notexample.com` is never a descendant of
//!    `example.com`.
//! 3. An [`Ipv4Destination`] is never an address a scan is forbidden to reach:
//!    RFC 1918 private space, loopback, link-local, multicast, the reserved
//!    blocks, the broadcast address, `0.0.0.0/8` and CGNAT `100.64.0.0/10`
//!    are all refused at construction.
//!
//! The module depends only on `idna`, `url`, `psl`, `ipnet`, `thiserror` and
//! `serde`; it performs no I/O and holds no state.
//!
//! Two rules the §A3.1 contract leaves open are decided here, fail closed, and
//! are the two places to look first when a corpus case disagrees:
//!
//! * **A suffix the Public Suffix List does not know is refused**
//!   ([`HostError::UnknownSuffix`]). `example.invalidtld` and `host.internal`
//!   are not scan targets, and the alternative — treating an unknown last
//!   label as a suffix — would silently make any typo a registrable name.
//!   A single-label input keeps its own reason ([`HostError::SingleLabel`]).
//! * **`ExactHost` plus `include_subdomains` narrows, it does not refuse.**
//!   The run is admitted for the root host alone, and every descendant
//!   discovery proposes is refused one by one
//!   ([`ScopeVerdict::OutsideApproval`]). §A3.1 says an `ExactHost` approval
//!   "permits only the identical host" and §A7.2 says an out-of-scope cascade
//!   candidate is recorded with a visible reason rather than silently dropped —
//!   refusing the whole run instead would hide which candidate was out of
//!   scope. The widening path is closed either way: no code path admits a
//!   non-identical host under `ExactHost`.
//!
//! * **`include_subdomains` governs the cascade, not the choice of run
//!   target.** A `DomainTree` approval admits any dot-boundary descendant as a
//!   run target on its own; `include_subdomains` then decides whether that run
//!   may *reach* descendants of its target. §A3.1 reads "a run includes
//!   descendants only when `include_subdomains` is true", and §A7.2 row 2
//!   pairs that sentence with "a cascade candidate outside scope is recorded
//!   `Skipped{Scope}`" — both sentences are about the cascade fan-out
//!   ([`check_candidate`], §A3.5 `derive_candidates`), which is where the
//!   switch is applied. Reading it as a rule about the run target instead
//!   would make "scan exactly `www.example.com`" impossible under a
//!   `DomainTree` approval of `example.com` without also switching discovery
//!   on, so the narrow request would be the one that forces the wide run —
//!   and a run whose own target failed [`check_run_scope`] could then scan
//!   nothing at all.
//!
//! A fourth rule is decided by what the module cannot observe rather than by
//! §A3.1: the IPv4 blocks counted as "reserved". `RESERVED_BLOCKS` enumerates
//! them, and the enumeration is deliberately wider than class E — a
//! documentation or benchmarking block is not a scan target either.

use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

use idna::uts46::{AsciiDenyList, DnsLength, Hyphens, Uts46};
use ipnet::Ipv4Net;

/// The DNS limit on the whole name, in bytes of its A-label form.
const MAX_NAME_BYTES: usize = 253;

/// The DNS limit on one label, in bytes of its A-label form.
const MAX_LABEL_BYTES: usize = 63;

/// Reason a host or destination was refused.
///
/// One variant per rejection, as required by the C2 contract: a refusal is
/// surfaced to the operator and recorded in the audit trail, so "invalid host"
/// is not an acceptable answer anywhere in this module.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HostError {
    /// Input was empty, or consisted only of the root dot.
    #[error("host is empty")]
    Empty,

    /// Input had a single label (`localhost`, `internal`); a scan target is
    /// always a registrable name under a public suffix.
    #[error("host has a single label and no public suffix")]
    SingleLabel,

    /// Input contained an empty label (`a..b`, `.a.b`).
    #[error("host contains an empty label")]
    EmptyLabel,

    /// A DNS label exceeded 63 bytes in A-label form.
    #[error("host contains a label of {len} bytes; the DNS limit is 63")]
    LabelTooLong {
        /// Length of the offending label, in bytes of its A-label form.
        len: usize,
    },

    /// The whole name exceeded 253 bytes in A-label form.
    #[error("host is {len} bytes; the DNS limit is 253")]
    TooLong {
        /// Length of the A-label form, in bytes.
        len: usize,
    },

    /// A label started or ended with `-`.
    #[error("host contains a label starting or ending with a hyphen")]
    HyphenAtLabelEdge,

    /// Input carried a port (`example.com:8443`).
    #[error("host carries a port; a port belongs in the operation scope, not the host")]
    PortNotAllowed,

    /// Input carried a scheme, path, query or fragment.
    #[error("host carries a scheme, path, query or fragment")]
    PathNotAllowed,

    /// Input carried userinfo (`user@example.com`).
    #[error("host carries userinfo")]
    UserInfoNotAllowed,

    /// Input was percent-encoded; a host is given in its decoded form.
    #[error("host is percent-encoded")]
    PercentEncoded,

    /// Input contained a character that is not allowed in a DNS label
    /// (UTS-46 `UseSTD3ASCIIRules`: letters, digits and hyphen only).
    #[error("host contains the forbidden character {character:?}")]
    ForbiddenCharacter {
        /// The first offending character.
        character: char,
    },

    /// Input was an IP literal, bracketed or not.
    #[error("host is an IP literal; use Ipv4Destination for an address")]
    IpLiteral,

    /// Input's last label was all digits, which the WHATWG host parser treats
    /// as a malformed IPv4 address rather than a domain.
    #[error("host's last label is all digits")]
    NumericLastLabel,

    /// UTS-46 (non-transitional) processing refused the input: invalid
    /// punycode, a disallowed code point, a script-mixing or bidi violation, or
    /// a misplaced joiner.
    #[error("host failed IDNA UTS-46 processing")]
    Idna,

    /// The A-label form did not round-trip through the WHATWG host parser as a
    /// domain. Defence in depth: it means `url` and `idna` disagree about the
    /// value, so no downstream URL built from it would be trustworthy.
    #[error("host is not a domain under the WHATWG host parser")]
    NotADomain,

    /// Input was itself a public suffix, ICANN or private section
    /// (`com`, `co.uk`, `github.io`). Approving a suffix root would approve
    /// every registrant under it.
    #[error("host is the public-suffix root {suffix:?}")]
    PublicSuffixRoot {
        /// The matched public suffix.
        suffix: String,
    },

    /// The Public Suffix List yielded no suffix for the input at all.
    #[error("host has no public suffix")]
    UnknownSuffix,

    /// Input was not a dotted-quad IPv4 address.
    #[error("destination is not an IPv4 address")]
    NotAnIpv4Address,

    /// RFC 1918 private space: `10/8`, `172.16/12`, `192.168/16`.
    #[error("destination {addr} is RFC 1918 private space")]
    PrivateAddress {
        /// The refused address.
        addr: Ipv4Addr,
    },

    /// Loopback: `127.0.0.0/8`.
    #[error("destination {addr} is loopback")]
    LoopbackAddress {
        /// The refused address.
        addr: Ipv4Addr,
    },

    /// Link-local: `169.254.0.0/16`.
    #[error("destination {addr} is link-local")]
    LinkLocalAddress {
        /// The refused address.
        addr: Ipv4Addr,
    },

    /// Multicast: `224.0.0.0/4`.
    #[error("destination {addr} is multicast")]
    MulticastAddress {
        /// The refused address.
        addr: Ipv4Addr,
    },

    /// Limited broadcast: `255.255.255.255`.
    #[error("destination {addr} is the broadcast address")]
    BroadcastAddress {
        /// The refused address.
        addr: Ipv4Addr,
    },

    /// "This network": `0.0.0.0/8`, including the unspecified address.
    #[error("destination {addr} is in 0.0.0.0/8")]
    ThisNetwork {
        /// The refused address.
        addr: Ipv4Addr,
    },

    /// CGNAT shared address space: `100.64.0.0/10`.
    #[error("destination {addr} is CGNAT shared address space 100.64.0.0/10")]
    SharedAddressSpace {
        /// The refused address.
        addr: Ipv4Addr,
    },

    /// A reserved or special-purpose block that is never a scan target.
    #[error("destination {addr} is in the reserved block {block} ({purpose})")]
    ReservedAddress {
        /// The refused address.
        addr: Ipv4Addr,
        /// The CIDR block it fell in.
        block: &'static str,
        /// Why that block is reserved.
        purpose: &'static str,
    },
}

/// A DNS domain name in canonical A-label form.
///
/// Construct with [`CanonicalHost::parse`]. The inner string is private
/// because the invariants in the module documentation are only true of values
/// that went through that constructor; `serde` deserialization goes through it
/// too, so a host read back from the database or an API body is re-validated
/// against the current Public Suffix List rather than trusted.
#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(try_from = "String", into = "String")]
pub struct CanonicalHost {
    /// A-label form: lowercase, no trailing dot.
    label: String,
}

impl CanonicalHost {
    /// Canonicalizes an untrusted hostname.
    ///
    /// Lowercases, strips one trailing dot, maps to A-label form with IDNA
    /// UTS-46 non-transitional processing, and rejects IP literals, ports,
    /// paths, empty labels, over-long labels and public-suffix roots.
    ///
    /// # Errors
    ///
    /// One [`HostError`] variant per rejection reason; see that enum.
    pub fn parse(input: &str) -> Result<CanonicalHost, HostError> {
        let trimmed = reject_non_dns_syntax(input)?;
        let ascii = to_a_labels(trimmed)?;
        check_dns_labels(&ascii)?;
        check_not_an_address(&ascii)?;
        check_not_a_suffix_root(&ascii)?;
        Ok(CanonicalHost { label: ascii })
    }

    /// The canonical A-label form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.label
    }

    /// The registrable domain (public suffix plus one label), via the Public
    /// Suffix List including its private section.
    ///
    /// Informational only. TDD §5.3 forbids using it to broaden an
    /// [`ScopeType::ExactHost`] approval, and nothing in this module does.
    #[must_use]
    pub fn registrable_domain(&self) -> Option<CanonicalHost> {
        let domain = psl::domain(self.label.as_bytes())?;
        let text = std::str::from_utf8(domain.as_bytes()).ok()?;
        if !domain.suffix().is_known() {
            return None;
        }
        // Derived from a value that already satisfies every invariant, by
        // cutting labels off its front: the result is canonical by
        // construction, and it is suffix-plus-one so it is not a suffix root.
        Some(CanonicalHost {
            label: text.to_owned(),
        })
    }

    /// Whether `self` is a strict descendant of `root` on a dot boundary.
    ///
    /// `false` when `self == root`, and `false` for a label prefix: with
    /// `root = example.com`, `notexample.com` is not a descendant.
    #[must_use]
    pub fn is_descendant_of(&self, root: &CanonicalHost) -> bool {
        let child = self.label.as_bytes();
        let parent = root.label.as_bytes();

        // Strict: a host is not a descendant of itself, so the child must be
        // longer by at least one label and one separator.
        let Some(boundary) = child.len().checked_sub(parent.len() + 1) else {
            return false;
        };
        child.get(boundary) == Some(&b'.') && child.ends_with(parent)
    }
}

impl fmt::Display for CanonicalHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}

impl FromStr for CanonicalHost {
    type Err = HostError;

    fn from_str(input: &str) -> Result<CanonicalHost, HostError> {
        CanonicalHost::parse(input)
    }
}

impl TryFrom<String> for CanonicalHost {
    type Error = HostError;

    fn try_from(input: String) -> Result<CanonicalHost, HostError> {
        CanonicalHost::parse(&input)
    }
}

impl From<CanonicalHost> for String {
    fn from(host: CanonicalHost) -> String {
        host.label
    }
}

/// Refuses everything that is URL syntax rather than a host, and strips the
/// root dot.
///
/// Runs before IDNA so that a refusal names the syntax that was present:
/// UTS-46 would either pass these characters through or collapse all of them
/// into one opaque processing failure.
fn reject_non_dns_syntax(input: &str) -> Result<&str, HostError> {
    if input.is_empty() {
        return Err(HostError::Empty);
    }

    // Percent-encoding first: its decoded form could hide any of the syntax
    // below, so it is refused rather than decoded.
    if input.contains('%') {
        return Err(HostError::PercentEncoded);
    }

    // Brackets before the colon check, so `[::1]` is reported as the address it
    // is rather than as a host carrying a port.
    if input.contains('[') || input.contains(']') {
        return Err(HostError::IpLiteral);
    }

    if input.contains("://") || input.contains(['/', '\\', '?', '#']) {
        return Err(HostError::PathNotAllowed);
    }

    if input.contains('@') {
        return Err(HostError::UserInfoNotAllowed);
    }

    if input.contains(':') {
        return Err(if input.parse::<Ipv6Addr>().is_ok() {
            HostError::IpLiteral
        } else {
            HostError::PortNotAllowed
        });
    }

    // One trailing dot is the DNS root label and is dropped. A second one is
    // deliberately left in place, so `example.com..` is reported as the empty
    // label it contains instead of being silently accepted.
    let trimmed = input.strip_suffix('.').unwrap_or(input);
    if trimmed.is_empty() {
        return Err(HostError::Empty);
    }

    Ok(trimmed)
}

/// Maps to A-label form with IDNA UTS-46 non-transitional processing.
///
/// `idna` 1.x is non-transitional, which is what §A3.1 asks for: `ß` and `ς`
/// keep their own identity instead of folding onto `ss` and `σ`, so two
/// different owners' names cannot canonicalize onto one another.
///
/// The ASCII deny list and the hyphen and length checks are switched off here
/// and applied by [`check_dns_labels`] to the A-label output instead. Each of
/// those refusals has its own [`HostError`] variant, and `idna` reports all of
/// them as one undifferentiated processing failure — running them afterwards is
/// what lets a refusal name the offending label or character.
fn to_a_labels(input: &str) -> Result<String, HostError> {
    Uts46::new()
        .to_ascii(
            input.as_bytes(),
            AsciiDenyList::EMPTY,
            Hyphens::Allow,
            DnsLength::Ignore,
        )
        .map(|ascii| ascii.into_owned())
        .map_err(|_| HostError::Idna)
}

/// Applies the DNS length and charset rules to an A-label form.
///
/// The charset is `UseSTD3ASCIIRules`, spelled out here rather than delegated
/// to `idna`'s deny list: letters, digits and hyphen, with no hyphen at either
/// edge of a label. Because the input is already A-label form, every non-ASCII
/// code point has become Punycode, so a character that survives to this point
/// and is outside that set was ASCII in the input and can be named.
fn check_dns_labels(ascii: &str) -> Result<(), HostError> {
    if ascii.is_empty() {
        return Err(HostError::Empty);
    }
    if ascii.len() > MAX_NAME_BYTES {
        return Err(HostError::TooLong { len: ascii.len() });
    }

    for label in ascii.split('.') {
        if label.is_empty() {
            return Err(HostError::EmptyLabel);
        }
        if label.len() > MAX_LABEL_BYTES {
            return Err(HostError::LabelTooLong { len: label.len() });
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err(HostError::HyphenAtLabelEdge);
        }

        // Uppercase is included in the refusal set on purpose: UTS-46 case
        // folds, so an uppercase byte here would mean the A-label form is not
        // canonical and comparisons downstream would be case sensitive.
        let forbidden = label
            .chars()
            .find(|c| !c.is_ascii_lowercase() && !c.is_ascii_digit() && *c != '-');
        if let Some(character) = forbidden {
            return Err(HostError::ForbiddenCharacter { character });
        }
    }

    Ok(())
}

/// Refuses an A-label form that is an address rather than a name.
fn check_not_an_address(ascii: &str) -> Result<(), HostError> {
    // The IPv6 arm is redundant by construction — every textual IPv6 form
    // contains a colon, and `reject_non_dns_syntax` has already refused those —
    // and is kept so that this function is correct on its own rather than only
    // in the order `CanonicalHost::parse` happens to call it in.
    if ascii.parse::<Ipv4Addr>().is_ok() || ascii.parse::<Ipv6Addr>().is_ok() {
        return Err(HostError::IpLiteral);
    }

    // The WHATWG host parser reads a trailing all-digit label as an IPv4
    // address, so `foo.1` is a malformed address to every URL built from it
    // even though it is a well-formed DNS name. Refused here, with its own
    // reason, rather than left to the round trip below.
    let last_label = ascii.rsplit('.').next().unwrap_or(ascii);
    if !last_label.is_empty() && last_label.bytes().all(|b| b.is_ascii_digit()) {
        return Err(HostError::NumericLastLabel);
    }

    // Defence in depth: `url` and `idna` must agree that this value is a
    // domain and that it is already in its canonical form. A disagreement
    // means a URL built from the stored host would not address what the
    // approval named — threat model row "annotation differs from actual
    // target" — so it fails closed.
    match url::Host::parse(ascii) {
        Ok(url::Host::Domain(domain)) if domain == ascii => Ok(()),
        Ok(url::Host::Ipv4(_) | url::Host::Ipv6(_)) => Err(HostError::IpLiteral),
        Ok(url::Host::Domain(_)) | Err(_) => Err(HostError::NotADomain),
    }
}

/// Refuses an A-label form that is itself a public suffix, or that sits under
/// no suffix the Public Suffix List knows.
///
/// Both sections of the list count: `github.io` is a private-section suffix,
/// and approving it would approve every GitHub Pages site at once.
fn check_not_a_suffix_root(ascii: &str) -> Result<(), HostError> {
    if !ascii.contains('.') {
        return Err(HostError::SingleLabel);
    }

    let Some(suffix) = psl::suffix(ascii.as_bytes()) else {
        return Err(HostError::UnknownSuffix);
    };

    // An unknown suffix is the list's wildcard default rule, not a fact about
    // the name. Treating it as a suffix would make every typo and every
    // internal name (`host.internal`) a registrable domain.
    if !suffix.is_known() {
        return Err(HostError::UnknownSuffix);
    }

    // A suffix is always a trailing substring of the name, so equal lengths
    // mean the name is the suffix.
    if suffix.as_bytes().len() == ascii.len() {
        return Err(HostError::PublicSuffixRoot {
            suffix: String::from_utf8_lossy(suffix.as_bytes()).into_owned(),
        });
    }

    Ok(())
}

/// How wide an authorization basis reaches — TDD §5.6 `scope_type`.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ScopeType {
    /// The root and its dot-boundary descendants.
    DomainTree,
    /// The root host and nothing else.
    ExactHost,
}

/// A `scope_type` column value that is not one of the two §5.6 variants.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("unknown scope_type {value:?}")]
pub struct UnknownScopeType {
    /// The value that was read.
    pub value: String,
}

impl ScopeType {
    /// The §5.6 `scope_type` column value.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            ScopeType::DomainTree => "domain_tree",
            ScopeType::ExactHost => "exact_host",
        }
    }
}

impl fmt::Display for ScopeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ScopeType {
    type Err = UnknownScopeType;

    fn from_str(value: &str) -> Result<ScopeType, UnknownScopeType> {
        match value {
            "domain_tree" => Ok(ScopeType::DomainTree),
            "exact_host" => Ok(ScopeType::ExactHost),
            other => Err(UnknownScopeType {
                value: other.to_owned(),
            }),
        }
    }
}

/// What the owner approved: a scope type and the root it applies to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovedScope {
    /// Width of the approval.
    pub scope_type: ScopeType,
    /// Host the approval is anchored on.
    pub root: CanonicalHost,
}

/// What a pipeline run asked for inside that approval.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunScope {
    /// The host the run was submitted against.
    pub target: CanonicalHost,
    /// Whether discovery may expand to descendants of `target`.
    pub include_subdomains: bool,
}

/// Outcome of a scope check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ScopeVerdict {
    /// Permitted by both the approval and the run scope.
    InScope,
    /// Not covered by the approval.
    OutsideApproval,
    /// Covered by the approval, but the run did not select discovery, so a
    /// descendant may not be reached.
    DiscoveryNotSelected,
    /// The approval root, the run target or the candidate is a public suffix
    /// under the current list, so nothing may be derived from it.
    PublicSuffix,
}

/// Whether a run's target is permitted by the approval.
///
/// An `ExactHost` approval permits only the identical host. `include_subdomains`
/// is not consulted here: under `ExactHost` it cannot widen anything, because
/// [`check_candidate`] tests every candidate against the approval first, so a
/// run that asked for discovery it will never be granted is admitted for its
/// root host and refused candidate by candidate (module documentation).
///
/// A `DomainTree` approval permits its root and any dot-boundary descendant as
/// a run target; whether the run may then *reach* descendants of that target is
/// [`check_candidate`]'s question, not this one.
///
/// Both hosts are re-tested against the Public Suffix List, because a host
/// canonicalized under an older list can have become a suffix since
/// (`github.io` was added to the private section years after the list existed).
#[must_use]
pub fn check_run_scope(approval: &ApprovedScope, run: &RunScope) -> ScopeVerdict {
    if is_public_suffix_root(&approval.root) || is_public_suffix_root(&run.target) {
        return ScopeVerdict::PublicSuffix;
    }

    if approves(approval, &run.target) {
        ScopeVerdict::InScope
    } else {
        ScopeVerdict::OutsideApproval
    }
}

/// Whether the approval alone covers `host`, ignoring any run.
///
/// The one place the `ExactHost` / `DomainTree` width difference is expressed,
/// so [`check_run_scope`] and [`check_candidate`] cannot drift apart.
fn approves(approval: &ApprovedScope, host: &CanonicalHost) -> bool {
    match approval.scope_type {
        ScopeType::ExactHost => host == &approval.root,
        ScopeType::DomainTree => host == &approval.root || host.is_descendant_of(&approval.root),
    }
}

/// Whether a candidate host may be scanned under this approval and run.
///
/// Must pass the approval and the run scope; the approval is checked first, so
/// a candidate outside the approval reports [`ScopeVerdict::OutsideApproval`]
/// even when it is also a descendant the run did not select.
#[must_use]
pub fn check_candidate(
    approval: &ApprovedScope,
    run: &RunScope,
    candidate: &CanonicalHost,
) -> ScopeVerdict {
    if is_public_suffix_root(candidate) {
        return ScopeVerdict::PublicSuffix;
    }

    // The approval first, so a candidate the owner never approved reports
    // OutsideApproval even when it is also a descendant discovery did not ask
    // for. The reason an operator sees names the widest refusal.
    if !approves(approval, candidate) {
        return ScopeVerdict::OutsideApproval;
    }

    // Then the run: a candidate inside the approval still needs a run whose
    // own scope is valid, and which reaches it.
    let run_verdict = check_run_scope(approval, run);
    if run_verdict != ScopeVerdict::InScope {
        return run_verdict;
    }

    if candidate == &run.target {
        ScopeVerdict::InScope
    } else if candidate.is_descendant_of(&run.target) {
        if run.include_subdomains {
            ScopeVerdict::InScope
        } else {
            ScopeVerdict::DiscoveryNotSelected
        }
    } else {
        // Inside the approval but not under this run's target — a sibling
        // subtree. Another run may cover it; this one does not.
        ScopeVerdict::OutsideApproval
    }
}

/// Whether a host is itself a public suffix under the current list.
///
/// Defence in depth: [`CanonicalHost::parse`] already refuses a suffix root, so
/// this can only fire for a value canonicalized under an older list — exactly
/// the case where failing closed matters.
fn is_public_suffix_root(host: &CanonicalHost) -> bool {
    match psl::suffix(host.label.as_bytes()) {
        Some(suffix) => suffix.is_known() && suffix.as_bytes().len() == host.label.len(),
        None => false,
    }
}

/// An IPv4 block that is never a scan destination.
///
/// The blocks that have a [`HostError`] variant of their own — RFC 1918,
/// loopback, link-local, multicast, broadcast, `0.0.0.0/8` and CGNAT — are not
/// in this table: [`Ipv4Destination::new`] tests those first so that the
/// refusal an operator reads names the specific rule rather than "reserved".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReservedBlock {
    /// First address of the block.
    network: Ipv4Addr,
    /// Prefix length, in bits.
    prefix_len: u8,
    /// CIDR notation, carried into the refusal reason.
    cidr: &'static str,
    /// Why the block is reserved, carried into the refusal reason.
    purpose: &'static str,
}

impl ReservedBlock {
    /// Whether `addr` falls in this block.
    ///
    /// Containment goes through `ipnet` rather than hand-rolled mask
    /// arithmetic, so the prefix maths on a security decision is not this
    /// module's to get wrong. A prefix length `ipnet` refuses can only come
    /// from a malformed row in the `const` table below, and the answer then is
    /// "yes, in the block": a block this module cannot interpret must not
    /// become a destination.
    fn contains(&self, addr: Ipv4Addr) -> bool {
        match Ipv4Net::new(self.network, self.prefix_len) {
            Ok(block) => block.contains(&addr),
            Err(_) => true,
        }
    }
}

/// CGNAT shared address space (RFC 6598). Carried as a block rather than a
/// pair of comparisons because `Ipv4Addr::is_shared` is not stable.
const SHARED_ADDRESS_SPACE: ReservedBlock = ReservedBlock {
    network: Ipv4Addr::new(100, 64, 0, 0),
    prefix_len: 10,
    cidr: "100.64.0.0/10",
    purpose: "CGNAT shared address space",
};

/// Special-purpose IPv4 blocks that are never a scan destination.
///
/// Deliberately wider than class E: a documentation, benchmarking or protocol
/// assignment block is not a target either, and `Ipv4Addr::is_reserved` and
/// `is_benchmarking` are both unstable, so the enumeration is explicit. The
/// table is a `const`, so a row costs nothing at run time and cannot fail to
/// load.
const RESERVED_BLOCKS: &[ReservedBlock] = &[
    ReservedBlock {
        network: Ipv4Addr::new(192, 0, 0, 0),
        prefix_len: 24,
        cidr: "192.0.0.0/24",
        purpose: "IETF protocol assignments",
    },
    ReservedBlock {
        network: Ipv4Addr::new(192, 0, 2, 0),
        prefix_len: 24,
        cidr: "192.0.2.0/24",
        purpose: "TEST-NET-1 documentation",
    },
    ReservedBlock {
        network: Ipv4Addr::new(198, 51, 100, 0),
        prefix_len: 24,
        cidr: "198.51.100.0/24",
        purpose: "TEST-NET-2 documentation",
    },
    ReservedBlock {
        network: Ipv4Addr::new(203, 0, 113, 0),
        prefix_len: 24,
        cidr: "203.0.113.0/24",
        purpose: "TEST-NET-3 documentation",
    },
    ReservedBlock {
        network: Ipv4Addr::new(192, 88, 99, 0),
        prefix_len: 24,
        cidr: "192.88.99.0/24",
        purpose: "deprecated 6to4 relay anycast",
    },
    ReservedBlock {
        network: Ipv4Addr::new(198, 18, 0, 0),
        prefix_len: 15,
        cidr: "198.18.0.0/15",
        purpose: "inter-network benchmarking",
    },
    ReservedBlock {
        network: Ipv4Addr::new(240, 0, 0, 0),
        prefix_len: 4,
        cidr: "240.0.0.0/4",
        purpose: "reserved for future use",
    },
];

/// An IPv4 address a scan is allowed to reach.
///
/// Construct with [`Ipv4Destination::new`] or [`Ipv4Destination::parse`]. The
/// inner address is private so that the refusal list cannot be bypassed: there
/// is no `From<Ipv4Addr>` and no `serde` derive, because either one would be a
/// way to obtain the type without running the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ipv4Destination(Ipv4Addr);

impl Ipv4Destination {
    /// Accepts an address that is not in any refused block.
    ///
    /// The order below is the order the reasons are reported in, most specific
    /// first, so the refusal names the rule that applies rather than the widest
    /// block that happens to contain the address. `is_broadcast` comes before
    /// the reserved table for exactly that reason: `255.255.255.255` is also
    /// inside `240.0.0.0/4`.
    ///
    /// # Errors
    ///
    /// One [`HostError`] variant per refused block; see that enum.
    pub fn new(addr: Ipv4Addr) -> Result<Ipv4Destination, HostError> {
        if addr.octets()[0] == 0 {
            return Err(HostError::ThisNetwork { addr });
        }
        if addr.is_loopback() {
            return Err(HostError::LoopbackAddress { addr });
        }
        if addr.is_private() {
            return Err(HostError::PrivateAddress { addr });
        }
        if addr.is_link_local() {
            return Err(HostError::LinkLocalAddress { addr });
        }
        if SHARED_ADDRESS_SPACE.contains(addr) {
            return Err(HostError::SharedAddressSpace { addr });
        }
        if addr.is_multicast() {
            return Err(HostError::MulticastAddress { addr });
        }
        if addr.is_broadcast() {
            return Err(HostError::BroadcastAddress { addr });
        }
        if let Some(block) = RESERVED_BLOCKS.iter().find(|block| block.contains(addr)) {
            return Err(HostError::ReservedAddress {
                addr,
                block: block.cidr,
                purpose: block.purpose,
            });
        }

        Ok(Ipv4Destination(addr))
    }

    /// Parses a dotted-quad address, then applies [`Ipv4Destination::new`].
    ///
    /// # Errors
    ///
    /// [`HostError::NotAnIpv4Address`] when the input is not a dotted quad,
    /// otherwise whatever [`Ipv4Destination::new`] reports.
    pub fn parse(input: &str) -> Result<Ipv4Destination, HostError> {
        let addr = input
            .parse::<Ipv4Addr>()
            .map_err(|_| HostError::NotAnIpv4Address)?;
        Ipv4Destination::new(addr)
    }

    /// The accepted address.
    #[must_use]
    pub fn addr(&self) -> Ipv4Addr {
        self.0
    }
}
