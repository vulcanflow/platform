//! T2 (VFL-41): `Ipv4Destination` corpus — rejects RFC 1918, loopback, link-local,
//! multicast, reserved, broadcast, 0.0.0.0/8 and CGNAT 100.64/10 (architecture §A3.1).
//!
//! Finalized against Kelly's C2 skeleton-committed surface (VFL-15 comment
//! d64b4109, commit 4540052): `Ipv4Destination::{new, parse, addr}`, superseding
//! this file's earlier `TryFrom<Ipv4Addr>` guess (§A3.1 itself only showed the
//! private-field struct shape, not a constructor name). `new` takes the typed
//! `std::net::Ipv4Addr`; `parse` takes `&str` and applies the same validation,
//! consistent with `CanonicalHost::parse(&str)` elsewhere in this module; `addr`
//! is the getter back to the validated `Ipv4Addr`. Both constructors are run
//! through the full corpus so a divergence between the two paths is caught.

use std::net::Ipv4Addr;
use vf_core::scope::Ipv4Destination;

fn rejects(addr: &str) {
    let ip: Ipv4Addr = addr.parse().expect("test fixture address must parse");
    assert!(
        Ipv4Destination::new(ip).is_err(),
        "expected {addr} to be rejected as a scan destination via new()"
    );
    assert!(
        Ipv4Destination::parse(addr).is_err(),
        "expected {addr} to be rejected as a scan destination via parse()"
    );
}

fn accepts(addr: &str) {
    let ip: Ipv4Addr = addr.parse().expect("test fixture address must parse");
    let via_new = Ipv4Destination::new(ip)
        .unwrap_or_else(|e| panic!("expected {addr} to be accepted via new(), got {e:?}"));
    let via_parse = Ipv4Destination::parse(addr)
        .unwrap_or_else(|e| panic!("expected {addr} to be accepted via parse(), got {e:?}"));
    assert_eq!(
        via_new.addr(),
        ip,
        "new() must preserve the address through addr()"
    );
    assert_eq!(
        via_parse.addr(),
        ip,
        "parse() must preserve the address through addr()"
    );
}

#[test]
fn rejects_rfc1918_private_ranges() {
    rejects("10.0.0.1");
    rejects("10.255.255.254");
    rejects("172.16.0.1");
    rejects("172.31.255.254");
    rejects("192.168.0.1");
    rejects("192.168.255.254");
}

#[test]
fn rejects_loopback() {
    rejects("127.0.0.1");
    rejects("127.255.255.254");
}

#[test]
fn rejects_link_local() {
    rejects("169.254.0.1");
    rejects("169.254.255.254");
}

#[test]
fn rejects_multicast() {
    rejects("224.0.0.1");
    rejects("239.255.255.255");
}

#[test]
fn rejects_reserved_class_e() {
    rejects("240.0.0.1");
    rejects("255.255.255.254");
}

#[test]
fn rejects_reserved_ietf_protocol_assignments() {
    rejects("192.0.0.1");
}

#[test]
fn rejects_reserved_test_net_1() {
    rejects("192.0.2.1");
}

#[test]
fn rejects_reserved_test_net_2() {
    rejects("198.51.100.1");
}

#[test]
fn rejects_reserved_test_net_3() {
    rejects("203.0.113.1");
}

#[test]
fn rejects_reserved_6to4_relay_anycast() {
    rejects("192.88.99.1");
}

#[test]
fn rejects_reserved_inter_network_benchmarking() {
    rejects("198.18.0.1");
    // 198.18.0.0/15 spans 198.18.0.0-198.19.255.255; the upper edge is the one
    // sample a /15-to-/16 prefix-length typo in the production table would not
    // catch (VFL-384 MEDIUM-2).
    rejects("198.19.255.255");
}

#[test]
fn rejects_broadcast() {
    rejects("255.255.255.255");
}

#[test]
fn rejects_zero_slash_eight() {
    rejects("0.0.0.0");
    rejects("0.1.2.3");
    rejects("0.255.255.255");
}

#[test]
fn rejects_cgnat_shared_address_space() {
    rejects("100.64.0.1");
    rejects("100.127.255.254");
}

#[test]
fn accepts_ordinary_public_addresses() {
    accepts("93.184.216.34"); // example.com's long-standing public address
    accepts("8.8.8.8");
    accepts("1.1.1.1");
}

#[test]
fn accepts_public_address_immediately_outside_cgnat_block() {
    // 100.64.0.0/10 is 100.64.0.0-100.127.255.255; one below and one above that block.
    accepts("100.63.255.255");
    accepts("100.128.0.0");
}
