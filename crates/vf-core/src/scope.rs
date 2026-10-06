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
//! API skeleton (task C2, first commit): the types and signatures below are
//! exactly the §A3.1 contract. Every body that carries logic is `todo!()` and
//! is filled in by the following commit; test pack T2 is written against this
//! surface.

use std::fmt;
use std::net::Ipv4Addr;
use std::str::FromStr;

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
    /// punycode, a disallowed code point, a bidi or joiner violation, or a
    /// positional hyphen in a decoded label.
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
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Serialize, serde::Deserialize)]
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
    pub fn parse(input: &str) -> Result<CanonicalHost, HostError> {
        let _ = input;
        todo!("C2: CanonicalHost::parse")
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
        todo!("C2: CanonicalHost::registrable_domain")
    }

    /// Whether `self` is a strict descendant of `root` on a dot boundary.
    ///
    /// `false` when `self == root`, and `false` for a label prefix: with
    /// `root = example.com`, `notexample.com` is not a descendant.
    #[must_use]
    pub fn is_descendant_of(&self, root: &CanonicalHost) -> bool {
        let _ = root;
        todo!("C2: CanonicalHost::is_descendant_of")
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

/// How wide an authorization basis reaches — TDD §5.6 `scope_type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Serialize, serde::Deserialize)]
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
#[must_use]
pub fn check_run_scope(approval: &ApprovedScope, run: &RunScope) -> ScopeVerdict {
    let _ = (approval, run);
    todo!("C2: check_run_scope")
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
    let _ = (approval, run, candidate);
    todo!("C2: check_candidate")
}

/// An IPv4 address a scan is allowed to reach.
///
/// Construct with [`Ipv4Destination::new`] or [`Ipv4Destination::parse`]. The
/// inner address is private so that the refusal list cannot be bypassed, and
/// `serde` deserialization re-runs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ipv4Destination(Ipv4Addr);

impl Ipv4Destination {
    /// Accepts an address that is not in any refused block.
    pub fn new(addr: Ipv4Addr) -> Result<Ipv4Destination, HostError> {
        let _ = addr;
        todo!("C2: Ipv4Destination::new")
    }

    /// Parses a dotted-quad address, then applies [`Ipv4Destination::new`].
    pub fn parse(input: &str) -> Result<Ipv4Destination, HostError> {
        let _ = input;
        todo!("C2: Ipv4Destination::parse")
    }

    /// The accepted address.
    #[must_use]
    pub fn addr(&self) -> Ipv4Addr {
        self.0
    }
}
