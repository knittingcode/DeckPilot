//! Address policy for brokered connections (SSRF defence, design §4.5).
//!
//! Origins are granted by DNS name only, but a name can resolve to anything.
//! The network layer must resolve the name itself, check **every** address
//! it is about to connect to with [`check_connect_addr`], and connect to
//! that vetted address (not resolve again), so DNS rebinding cannot swap in
//! a private address after the check.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Why an address may not be connected to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockedAddr {
    Loopback,
    Private,
    LinkLocal,
    SharedCgnat,
    Unspecified,
    Multicast,
    Broadcast,
    Reserved,
    Documentation,
    Benchmarking,
    UniqueLocal,
    /// IPv6 transition forms that embed or tunnel an IPv4 address (6to4,
    /// Teredo, NAT64 with a blocked embedded address, IPv4-compatible).
    Tunnel,
}

/// Allows only globally routable unicast addresses.
pub fn check_connect_addr(ip: IpAddr) -> Result<(), BlockedAddr> {
    match ip {
        IpAddr::V4(v4) => check_v4(v4),
        IpAddr::V6(v6) => check_v6(v6),
    }
}

fn check_v4(ip: Ipv4Addr) -> Result<(), BlockedAddr> {
    let [a, b, c, _] = ip.octets();
    use BlockedAddr::*;
    match (a, b, c) {
        (0, _, _) => Err(Unspecified),
        (127, _, _) => Err(Loopback),
        (10, _, _) => Err(Private),
        (172, 16..=31, _) => Err(Private),
        (192, 168, _) => Err(Private),
        (169, 254, _) => Err(LinkLocal), // includes 169.254.169.254 (cloud metadata)
        (100, 64..=127, _) => Err(SharedCgnat),
        (192, 0, 0) => Err(Reserved), // IETF protocol assignments
        (192, 0, 2) | (198, 51, 100) | (203, 0, 113) => Err(Documentation),
        (198, 18..=19, _) => Err(Benchmarking),
        (192, 88, 99) => Err(Tunnel), // 6to4 relay anycast (deprecated)
        (224..=239, _, _) => Err(Multicast),
        (255, 255, 255) if ip == Ipv4Addr::BROADCAST => Err(Broadcast),
        (240..=255, _, _) => Err(Reserved),
        _ => Ok(()),
    }
}

fn check_v6(ip: Ipv6Addr) -> Result<(), BlockedAddr> {
    use BlockedAddr::*;
    if ip.is_unspecified() {
        return Err(Unspecified);
    }
    if ip.is_loopback() {
        return Err(Loopback);
    }
    // IPv4-mapped (::ffff:a.b.c.d): judge the IPv4 address.
    if let Some(v4) = ip.to_ipv4_mapped() {
        return check_v4(v4);
    }
    let s = ip.segments();
    match s[0] {
        0xff00..=0xffff => return Err(Multicast),
        0xfe80..=0xfebf => return Err(LinkLocal),
        0xfec0..=0xfeff => return Err(Reserved), // site-local (deprecated)
        0xfc00..=0xfdff => return Err(UniqueLocal),
        0x2002 => return Err(Tunnel), // 6to4
        _ => {}
    }
    // IPv4-compatible (::a.b.c.d, deprecated) and the rest of ::/8.
    if s[0] == 0 {
        return Err(Tunnel);
    }
    // NAT64 well-known prefix 64:ff9b::/96 and local-use 64:ff9b:1::/48:
    // judge the embedded IPv4 address.
    if s[0] == 0x0064 && s[1] == 0xff9b {
        let embedded = Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8);
        if s[2] == 1 {
            return Err(Private);
        }
        check_v4(embedded).map_err(|_| Tunnel)?;
        return Ok(());
    }
    if s[0] == 0x2001 && s[1] == 0 {
        return Err(Tunnel); // Teredo 2001::/32
    }
    if s[0] == 0x2001 && s[1] == 0x0db8 {
        return Err(Documentation);
    }
    if s[0] == 0x3fff && s[1] <= 0x0fff {
        return Err(Documentation); // 3fff::/20
    }
    if s[0] == 0x2001 && s[1] == 0x0002 && s[2] == 0 {
        return Err(Benchmarking); // 2001:2::/48
    }
    if s[0] == 0x0100 && s[1] == 0 && s[2] == 0 && s[3] == 0 {
        return Err(Reserved); // discard-only 100::/64
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn blocks_non_global_ranges() {
        for s in [
            "0.0.0.0",
            "0.1.2.3",
            "127.0.0.1",
            "127.255.255.254",
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "169.254.0.1",
            "100.64.0.1",
            "100.127.255.255",
            "192.0.0.8",
            "192.0.2.1",
            "198.51.100.7",
            "203.0.113.9",
            "198.18.0.1",
            "224.0.0.1",
            "239.255.255.250",
            "240.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "::ffff:127.0.0.1",
            "::ffff:10.1.2.3",
            "::ffff:169.254.169.254",
            "::127.0.0.1",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "fec0::1",
            "ff02::1",
            "2002:7f00:1::1",
            "2002:c0a8:101::1",
            "2001:0:4136:e378::1",
            "2001:db8::1",
            "3fff::1",
            "64:ff9b::7f00:1",
            "64:ff9b::a00:1",
            "64:ff9b:1::1",
            "100::1",
        ] {
            assert!(check_connect_addr(ip(s)).is_err(), "{s} must be blocked");
        }
    }

    #[test]
    fn allows_global_unicast() {
        for s in [
            "1.1.1.1",
            "8.8.8.8",
            "93.184.215.14",
            "172.15.255.255",
            "172.32.0.1",
            "100.63.255.255",
            "100.128.0.1",
            "2606:4700:4700::1111",
            "2a00:1450:4001::200e",
            "::ffff:1.1.1.1",
            "64:ff9b::101:101",
        ] {
            assert_eq!(check_connect_addr(ip(s)), Ok(()), "{s} must be allowed");
        }
    }
}
