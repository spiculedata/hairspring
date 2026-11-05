//! The client address that the rate limit (and, without tokens, the per-caller event-stream cap) is keyed by. It is
//! the connection's peer address, unless that peer is a proxy the operator has named as trusted; then it is taken from
//! `X-Forwarded-For`. The `Forwarded` header and `X-Real-IP` are never read.
use axum::http::HeaderMap;
use std::fmt;
use std::net::IpAddr;

/// Most bytes of `X-Forwarded-For` examined, counted from the right and across headers; needing more to find the
/// client means the peer address is used. Text further left than the first address that is not a trusted proxy is
/// never examined, so it is not counted. The total header size is not capped here: the HTTP server bounds it, and
/// falling back on a long header would let a client send one.
const MAX_FORWARDED_BYTES: usize = 1024;
/// Most entries examined from the right, across headers; needing more to find the client means the peer address is
/// used.
const MAX_FORWARDED_HOPS: usize = 16;
/// Narrowest prefix accepted for an IPv4 network.
const MIN_V4_PREFIX: u8 = 8;
/// Narrowest prefix accepted for an IPv6 network.
const MIN_V6_PREFIX: u8 = 32;
/// Longest part of a rejected entry repeated in an error message, in characters.
const SHOWN_ENTRY_CHARS: usize = 16;
const X_FORWARDED_FOR: &str = "x-forwarded-for";

/// Proxy addresses and networks whose `X-Forwarded-For` header is believed. Empty by default: then forwarding headers
/// are ignored and the client address is always the connection's peer address.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrustedProxies {
    nets: Vec<Net>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Net {
    addr: IpAddr,
    prefix: u8,
}

impl TrustedProxies {
    /// Parses a comma-separated list of addresses (`10.0.0.5`, `2001:db8::5`) and networks (`10.0.0.0/24`,
    /// `2001:db8::/64`). A blank list trusts no proxy. Refused: an empty entry, anything that is not an address or a
    /// network in that form, a network with bits set past its prefix, an IPv4-mapped or IPv4-compatible IPv6 entry
    /// (write it as IPv4), a network wider than /8 (IPv4) or /32 (IPv6), and any entry that is, or is a range starting
    /// at, the unspecified address (`0.0.0.0`, `::`, `0.0.0.0/8`, `::/96`), since trusting a range that broad would
    /// let a client choose the address it is counted under. A repeated entry is listed once.
    pub fn parse(spec: &str) -> Result<TrustedProxies, String> {
        if spec.trim().is_empty() {
            return Ok(TrustedProxies::default());
        }
        let mut nets: Vec<Net> = Vec::new();
        for entry in spec.split(',') {
            let net = Net::parse(entry.trim())?;
            if !nets.contains(&net) {
                nets.push(net);
            }
        }
        Ok(TrustedProxies { nets })
    }

    /// True when no proxy is trusted.
    pub fn is_empty(&self) -> bool {
        self.nets.is_empty()
    }

    /// Whether `ip` is one of the trusted proxies. An IPv4-mapped IPv6 address is treated as its IPv4 address.
    fn contains(&self, ip: IpAddr) -> bool {
        let ip = ip.to_canonical();
        self.nets.iter().any(|net| net.contains(ip))
    }

    /// The address to count a request under. If `peer` is not a trusted proxy, `peer`. Otherwise the right-most
    /// address in `X-Forwarded-For` (several such headers are read as one list, in order) that is not itself a trusted
    /// proxy. Entries are read from the right and reading stops at that address: anything further left is written by
    /// the client and is not examined, so it cannot cause a fallback. Falls back to `peer` when the header is
    /// missing, an entry read on the way is not a plain IP address, finding the client needs more than
    /// [`MAX_FORWARDED_BYTES`] bytes or [`MAX_FORWARDED_HOPS`] entries, or every entry is a trusted proxy.
    pub fn client_ip(&self, peer: IpAddr, headers: &HeaderMap) -> IpAddr {
        if !self.contains(peer) {
            return peer;
        }
        self.first_untrusted_hop(headers).unwrap_or(peer)
    }

    /// The right-most `X-Forwarded-For` entry that is not a trusted proxy, or `None` if there is none within the
    /// bounds or an entry on the way is unusable.
    fn first_untrusted_hop(&self, headers: &HeaderMap) -> Option<IpAddr> {
        let values: Vec<_> = headers.get_all(X_FORWARDED_FOR).iter().collect();
        let mut bytes = 0;
        let mut hops = 0;
        for value in values.into_iter().rev() {
            for entry in value.to_str().ok()?.rsplit(',') {
                hops += 1;
                bytes += entry.len();
                if hops > MAX_FORWARDED_HOPS || bytes > MAX_FORWARDED_BYTES {
                    return None;
                }
                let hop: IpAddr = entry.trim_matches([' ', '\t']).parse().ok()?;
                if !self.contains(hop) {
                    return Some(hop);
                }
            }
        }
        None
    }
}

impl Net {
    fn parse(entry: &str) -> Result<Net, String> {
        let shown = shown_entry(entry);
        let malformed = || {
            format!(
                "`{shown}` is not an IP address or a network such as 10.0.0.0/24 or 2001:db8::/64"
            )
        };
        let (addr, prefix) = match entry.split_once('/') {
            Some((addr, prefix)) => (addr, Some(prefix)),
            None => (entry, None),
        };
        let addr: IpAddr = addr.parse().map_err(|_| malformed())?;
        if let IpAddr::V6(v6) = addr {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return Err(format!(
                    "`{shown}` is an IPv4-mapped address; write it as {v4}"
                ));
            }
            if is_ipv4_compatible(v6) {
                return Err(format!(
                    "`{shown}` is an IPv4-compatible address (deprecated); write the IPv4 address instead"
                ));
            }
        }
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            None => max,
            Some(p) => parse_prefix(p)
                .filter(|p| *p <= max)
                .ok_or_else(malformed)?,
        };
        let spoofable =
            " so a client could choose the address it is counted under; name the proxy's own \
                         address or network";
        if prefix == 0 {
            return Err(format!("`{shown}` covers every address,{spoofable}"));
        }
        let min = if addr.is_ipv4() {
            MIN_V4_PREFIX
        } else {
            MIN_V6_PREFIX
        };
        if prefix < min {
            return Err(format!(
                "`{shown}` is too broad (the widest network accepted is /{MIN_V4_PREFIX} for IPv4 and \
                 /{MIN_V6_PREFIX} for IPv6),{spoofable}"
            ));
        }
        if addr.is_unspecified() {
            return Err(if prefix == max {
                format!("`{shown}` is the unspecified address,{spoofable}")
            } else {
                format!("`{shown}` is a range starting at the unspecified address,{spoofable}")
            });
        }
        let net = Net {
            addr: masked(addr, prefix),
            prefix,
        };
        if net.addr != addr {
            return Err(format!(
                "`{shown}` has bits set past its /{prefix} prefix; did you mean {}/{prefix}?",
                net.addr
            ));
        }
        Ok(net)
    }

    fn contains(&self, ip: IpAddr) -> bool {
        ip.is_ipv4() == self.addr.is_ipv4() && masked(ip, self.prefix) == self.addr
    }
}

/// `::a.b.c.d` other than `::` and `::1`: the deprecated IPv4-compatible form.
fn is_ipv4_compatible(v6: std::net::Ipv6Addr) -> bool {
    v6.octets()[..12].iter().all(|b| *b == 0) && !v6.is_unspecified() && !v6.is_loopback()
}

/// The start of an entry for an error message: at most [`SHOWN_ENTRY_CHARS`] characters, control characters escaped,
/// with `...` when cut, so that a value pasted in by mistake is not repeated in full.
fn shown_entry(entry: &str) -> String {
    let mut shown: String = entry
        .chars()
        .take(SHOWN_ENTRY_CHARS)
        .flat_map(|c| c.escape_debug())
        .collect();
    if entry.chars().count() > SHOWN_ENTRY_CHARS {
        shown.push_str("...");
    }
    shown
}

/// A prefix length written in plain decimal digits, without a sign or a leading zero.
fn parse_prefix(text: &str) -> Option<u8> {
    let plain = !text.is_empty()
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    plain.then(|| text.parse().ok()).flatten()
}

/// `ip` with every bit after the first `prefix` cleared.
fn masked(ip: IpAddr, prefix: u8) -> IpAddr {
    match ip {
        IpAddr::V4(v4) => {
            let mask = u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0);
            IpAddr::V4((u32::from(v4) & mask).into())
        }
        IpAddr::V6(v6) => {
            let mask = u128::MAX.checked_shl(128 - u32::from(prefix)).unwrap_or(0);
            IpAddr::V6((u128::from(v6) & mask).into())
        }
    }
}

impl fmt::Display for TrustedProxies {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, net) in self.nets.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{}/{}", net.addr, net.prefix)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn proxies(spec: &str) -> TrustedProxies {
        TrustedProxies::parse(spec).unwrap_or_else(|e| panic!("{spec}: {e}"))
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn xff(values: &[&str]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for v in values {
            h.append(X_FORWARDED_FOR, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    const PROXY: &str = "10.0.0.5";

    #[test]
    fn a_blank_list_trusts_nothing() {
        assert!(proxies("").is_empty());
        assert!(proxies("   ").is_empty());
    }

    #[test]
    fn addresses_and_networks_of_both_families_parse() {
        let p = proxies(" 10.0.0.5 , 192.168.0.0/24,2001:db8::5, 2001:db8:1::/48 ");
        assert_eq!(
            p.to_string(),
            "10.0.0.5/32, 192.168.0.0/24, 2001:db8::5/128, 2001:db8:1::/48"
        );
        for inside in [
            "10.0.0.5",
            "192.168.0.1",
            "192.168.0.255",
            "2001:db8::5",
            "2001:db8:1:ffff::1",
        ] {
            assert!(p.contains(ip(inside)), "{inside}");
        }
        for outside in [
            "10.0.0.6",
            "192.168.1.0",
            "2001:db8::6",
            "2001:db8:2::1",
            "::ffff:10.0.0.6",
        ] {
            assert!(!p.contains(ip(outside)), "{outside}");
        }
    }

    #[test]
    fn an_ipv4_mapped_peer_matches_its_ipv4_entry() {
        assert!(proxies(PROXY).contains(ip("::ffff:10.0.0.5")));
    }

    #[test]
    fn entries_that_would_trust_everyone_are_refused() {
        for spec in ["0.0.0.0/0", "::/0", "0.0.0.0", "::", "10.0.0.5,0.0.0.0/0"] {
            let err = TrustedProxies::parse(spec).unwrap_err();
            assert!(
                err.contains("every") || err.contains("unspecified"),
                "{spec}: {err}"
            );
        }
    }

    #[test]
    fn a_zero_prefix_on_any_address_is_refused() {
        let err = TrustedProxies::parse("10.0.0.0/0").unwrap_err();
        assert!(err.contains("every"), "{err}");
    }

    #[test]
    fn very_broad_networks_are_refused_with_the_minimum_named() {
        for spec in [
            "10.0.0.0/7",
            "128.0.0.0/1",
            "0.0.0.0/1",
            "2001::/16",
            "::/1",
            "8000::/31",
        ] {
            let err = TrustedProxies::parse(spec).unwrap_err();
            assert!(
                err.contains("/8") && err.contains("/32") && err.contains("broad"),
                "{spec}: {err}"
            );
            assert!(!err.contains("names the unspecified"), "{spec}: {err}");
        }
        for spec in ["10.0.0.0/8", "2001:db8::/32", "10.0.0.0/9"] {
            assert!(TrustedProxies::parse(spec).is_ok(), "{spec}");
        }
    }

    #[test]
    fn ranges_starting_at_the_unspecified_address_are_described_accurately() {
        for spec in ["0.0.0.0/8", "0.0.0.0/24", "::/96", "::/127"] {
            let err = TrustedProxies::parse(spec).unwrap_err();
            assert!(err.contains("range"), "{spec}: {err}");
            assert!(!err.contains("names the unspecified"), "{spec}: {err}");
            assert!(!err.contains("every address"), "{spec}: {err}");
        }
        for spec in ["0.0.0.0", "::"] {
            let err = TrustedProxies::parse(spec).unwrap_err();
            assert!(err.contains("is the unspecified address"), "{spec}: {err}");
        }
    }

    #[test]
    fn a_bad_entry_is_shortened_and_its_control_characters_escaped_in_the_error() {
        let long = format!("{}SECRETTAIL", "a".repeat(40));
        let err = TrustedProxies::parse(&long).unwrap_err();
        assert!(!err.contains("SECRETTAIL"), "{err}");
        assert!(err.len() < 200, "{err}");
        let err = TrustedProxies::parse("10.0.0.5\n\x1b[31mx").unwrap_err();
        assert!(!err.contains('\n') && !err.contains('\x1b'), "{err:?}");
        let err = TrustedProxies::parse("10.0.0.0/8, 10.0.0.5/33").unwrap_err();
        assert!(
            err.contains("10.0.0.5/33"),
            "short entries are still shown: {err}"
        );
    }

    #[test]
    fn duplicate_entries_are_listed_once() {
        let p = proxies(
            "10.0.0.5, 10.0.0.5/32, 10.0.0.0/24, 10.0.0.0/24, 2001:db8::/32, 2001:db8::/32",
        );
        assert_eq!(p.to_string(), "10.0.0.5/32, 10.0.0.0/24, 2001:db8::/32");
    }

    #[test]
    fn ipv4_compatible_ipv6_entries_are_refused() {
        for spec in ["::1.2.3.4", "::102:304", "::10.0.0.5/128", "::a00:5"] {
            let err = TrustedProxies::parse(spec).unwrap_err();
            assert!(err.contains("IPv4-compatible"), "{spec}: {err}");
        }
        assert!(TrustedProxies::parse("::1").is_ok(), "loopback stays valid");
    }

    #[test]
    fn malformed_entries_are_refused() {
        for spec in [
            "10.0.0.5,",
            ",10.0.0.5",
            "10.0.0.5,,10.0.0.6",
            "10.0.0",
            "010.0.0.5",
            "10.0.0.5/",
            "10.0.0.5/33",
            "2001:db8::/129",
            "10.0.0.0/+8",
            "10.0.0.0/08",
            "10.0.0.0/ 8",
            "10.0.0.0/8/8",
            "10.0.0.0 /8",
            "localhost",
            "[2001:db8::1]",
            "10.0.0.5:80",
            "fe80::1%eth0",
            "::ffff:10.0.0.5",
        ] {
            assert!(
                TrustedProxies::parse(spec).is_err(),
                "{spec:?} was accepted"
            );
        }
    }

    #[test]
    fn a_network_with_host_bits_set_is_refused_with_a_suggestion() {
        let err = TrustedProxies::parse("10.0.0.5/24").unwrap_err();
        assert!(err.contains("10.0.0.0/24"), "{err}");
        assert!(TrustedProxies::parse("2001:db8::1/64").is_err());
    }

    #[test]
    fn an_untrusted_peer_is_used_whatever_it_forwards() {
        let p = proxies(PROXY);
        let peer = ip("192.0.2.9");
        assert_eq!(p.client_ip(peer, &xff(&["198.51.100.1"])), peer);
        assert_eq!(
            TrustedProxies::default().client_ip(ip(PROXY), &xff(&["198.51.100.1"])),
            ip(PROXY)
        );
    }

    #[test]
    fn a_trusted_peer_forwards_the_right_most_address() {
        let p = proxies(PROXY);
        let got = p.client_ip(ip(PROXY), &xff(&["203.0.113.66, 198.51.100.1"]));
        assert_eq!(
            got,
            ip("198.51.100.1"),
            "the left-most value is never trusted"
        );
    }

    #[test]
    fn chained_trusted_proxies_are_skipped_from_the_right() {
        let p = proxies("10.0.0.5, 10.1.0.0/16");
        let got = p.client_ip(
            ip(PROXY),
            &xff(&["203.0.113.66, 198.51.100.1, 10.1.2.3, 10.0.0.5"]),
        );
        assert_eq!(got, ip("198.51.100.1"));
    }

    #[test]
    fn only_trusted_hops_fall_back_to_the_peer() {
        let p = proxies("10.0.0.5, 10.1.0.0/16");
        assert_eq!(
            p.client_ip(ip(PROXY), &xff(&["10.1.2.3, 10.0.0.5"])),
            ip(PROXY)
        );
    }

    #[test]
    fn a_missing_or_empty_header_falls_back_to_the_peer() {
        let p = proxies(PROXY);
        assert_eq!(p.client_ip(ip(PROXY), &HeaderMap::new()), ip(PROXY));
        assert_eq!(p.client_ip(ip(PROXY), &xff(&[""])), ip(PROXY));
    }

    #[test]
    fn a_malformed_right_most_entry_falls_back_to_the_peer() {
        let p = proxies(PROXY);
        for bad in [
            "unknown",
            "198.51.100.1:4000",
            "[2001:db8::1]",
            "198.51.100.1,",
            "198.51.100.2, garbage",
            "198.51.100.1;198.51.100.2",
            "for=198.51.100.1",
            "fe80::1%eth0",
        ] {
            assert_eq!(p.client_ip(ip(PROXY), &xff(&[bad])), ip(PROXY), "{bad:?}");
        }
    }

    #[test]
    fn entries_left_of_the_first_untrusted_hop_are_never_examined() {
        let p = proxies("10.0.0.5, 10.1.0.0/16");
        let client = ip("198.51.100.1");
        for left in [
            "garbage",
            "",
            "a;b",
            "[2001:db8::1]",
            "999.1.1.1",
            "203.0.113.66:80",
            "not-an-address, , ,",
        ] {
            let header = format!("{left}, 198.51.100.1");
            assert_eq!(p.client_ip(ip(PROXY), &xff(&[&header])), client, "{left:?}");
            let header = format!("{left}, 198.51.100.1, 10.1.2.3");
            assert_eq!(p.client_ip(ip(PROXY), &xff(&[&header])), client, "{left:?}");
        }
        // A whole earlier header is left alone too, even one that is not text.
        let mut h = HeaderMap::new();
        h.append(
            X_FORWARDED_FOR,
            HeaderValue::from_bytes(b"\xff\xfe").unwrap(),
        );
        h.append(X_FORWARDED_FOR, HeaderValue::from_static("198.51.100.1"));
        assert_eq!(p.client_ip(ip(PROXY), &h), client);
    }

    #[test]
    fn a_long_or_crowded_left_side_does_not_cause_the_fallback() {
        let p = proxies(PROXY);
        let junk = "x".repeat(100_000);
        let header = format!("{junk}, 198.51.100.1");
        assert_eq!(p.client_ip(ip(PROXY), &xff(&[&header])), ip("198.51.100.1"));
        let many = format!("{}, 198.51.100.1", hops(500));
        assert_eq!(p.client_ip(ip(PROXY), &xff(&[&many])), ip("198.51.100.1"));
    }

    #[test]
    fn a_malformed_entry_met_while_skipping_trusted_hops_falls_back_to_the_peer() {
        // The malformed entry is the first one that must be read to find the client.
        let p = proxies("10.0.0.5, 10.1.0.0/16");
        assert_eq!(
            p.client_ip(ip(PROXY), &xff(&["198.51.100.1, garbage, 10.1.2.3"])),
            ip(PROXY)
        );
    }

    #[test]
    fn a_header_that_is_not_text_falls_back_to_the_peer() {
        let p = proxies(PROXY);
        let mut h = HeaderMap::new();
        h.append(
            X_FORWARDED_FOR,
            HeaderValue::from_bytes(b"198.51.100.\xff").unwrap(),
        );
        assert_eq!(p.client_ip(ip(PROXY), &h), ip(PROXY));
    }

    #[test]
    fn spaces_and_tabs_around_entries_are_allowed() {
        let p = proxies(PROXY);
        assert_eq!(
            p.client_ip(ip(PROXY), &xff(&[" 198.51.100.1 ,\t198.51.100.2\t"])),
            ip("198.51.100.2")
        );
    }

    #[test]
    fn several_headers_are_read_as_one_list_in_order() {
        let p = proxies(PROXY);
        let got = p.client_ip(ip(PROXY), &xff(&["203.0.113.66", "198.51.100.1, 10.0.0.5"]));
        assert_eq!(got, ip("198.51.100.1"));
        let got = p.client_ip(ip(PROXY), &xff(&["198.51.100.1, 10.0.0.5", "203.0.113.66"]));
        assert_eq!(got, ip("203.0.113.66"));
    }

    #[test]
    fn a_malformed_right_most_header_among_several_falls_back_to_the_peer() {
        let p = proxies(PROXY);
        assert_eq!(
            p.client_ip(ip(PROXY), &xff(&["198.51.100.1", "nonsense"])),
            ip(PROXY)
        );
        // Garbage in an earlier header is not read.
        assert_eq!(
            p.client_ip(ip(PROXY), &xff(&["nonsense", "198.51.100.1"])),
            ip("198.51.100.1")
        );
    }

    fn hops(n: usize) -> String {
        (0..n)
            .map(|i| format!("198.51.100.{}", i % 250))
            .collect::<Vec<_>>()
            .join(",")
    }

    fn trusted_hops(n: usize) -> String {
        (0..n)
            .map(|i| format!("10.1.0.{}", i % 250))
            .collect::<Vec<_>>()
            .join(",")
    }

    #[test]
    fn the_examined_hop_count_is_bounded() {
        let p = proxies("10.0.0.5, 10.1.0.0/16");
        let client = "198.51.100.77";
        // The client is the MAX_FORWARDED_HOPS-th entry from the right: still found.
        let header = format!("{client},{}", trusted_hops(MAX_FORWARDED_HOPS - 1));
        assert_eq!(p.client_ip(ip(PROXY), &xff(&[&header])), ip(client));
        // One more trusted hop in front of it: too many entries to examine.
        let header = format!("{client},{}", trusted_hops(MAX_FORWARDED_HOPS));
        assert_eq!(p.client_ip(ip(PROXY), &xff(&[&header])), ip(PROXY));
        // The bound counts across headers too.
        let half = trusted_hops(MAX_FORWARDED_HOPS / 2 + 1);
        assert_eq!(
            p.client_ip(ip(PROXY), &xff(&[client, &half, &half])),
            ip(PROXY)
        );
    }

    #[test]
    fn the_examined_length_is_bounded() {
        let p = proxies("10.0.0.5, 10.1.0.0/16");
        let pad = |n: usize, a: &str| format!("{}{a}", " ".repeat(n - a.len()));
        assert_eq!(
            p.client_ip(
                ip(PROXY),
                &xff(&[&pad(MAX_FORWARDED_BYTES, "198.51.100.1")])
            ),
            ip("198.51.100.1")
        );
        assert_eq!(
            p.client_ip(
                ip(PROXY),
                &xff(&[&pad(MAX_FORWARDED_BYTES + 1, "198.51.100.1")])
            ),
            ip(PROXY)
        );
        // The bound counts across headers too: a trusted hop and the client, together over the limit.
        let half = MAX_FORWARDED_BYTES / 2 + 1;
        assert_eq!(
            p.client_ip(
                ip(PROXY),
                &xff(&[&pad(half, "198.51.100.1"), &pad(half, "10.1.2.3")])
            ),
            ip(PROXY)
        );
    }

    #[test]
    fn ipv6_clients_and_proxies_work() {
        let p = proxies("2001:db8::5");
        let got = p.client_ip(ip("2001:db8::5"), &xff(&["2001:db8:aaaa:1::7"]));
        assert_eq!(got, ip("2001:db8:aaaa:1::7"));
    }

    #[test]
    fn an_ipv4_mapped_forwarded_address_counts_as_ipv4() {
        let p = proxies("10.0.0.5, 10.1.0.0/16");
        let got = p.client_ip(ip(PROXY), &xff(&["198.51.100.1, ::ffff:10.1.2.3"]));
        assert_eq!(got, ip("198.51.100.1"), "the mapped hop is a trusted proxy");
    }

    #[test]
    fn the_forwarded_and_x_real_ip_headers_are_not_read() {
        let p = proxies(PROXY);
        let mut h = HeaderMap::new();
        h.append("forwarded", HeaderValue::from_static("for=198.51.100.1"));
        h.append("x-real-ip", HeaderValue::from_static("198.51.100.2"));
        assert_eq!(p.client_ip(ip(PROXY), &h), ip(PROXY));
    }
}
