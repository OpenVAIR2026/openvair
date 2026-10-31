//! Which client address a request came from. Forwarding headers are only
//! believed from the reverse proxies in `TRUSTED_PROXIES`; anyone else could
//! write any address into them to dodge login limits or fake audit records.

use axum::{
    extract::{ConnectInfo, Request},
    http::HeaderValue,
    middleware::Next,
    response::Response,
};
use std::{
    net::{IpAddr, SocketAddr},
    sync::OnceLock,
};

const DEFAULT_TRUSTED: &str =
    "127.0.0.0/8,::1/128,10.0.0.0/8,172.16.0.0/12,192.168.0.0/16,fc00::/7";

static TRUSTED: OnceLock<Vec<Cidr>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cidr {
    network: IpAddr,
    prefix: u8,
}

impl Cidr {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        let (address, prefix) = match value.split_once('/') {
            Some((address, prefix)) => (address, Some(prefix.parse::<u8>().ok()?)),
            None => (value, None),
        };
        let network = canonical(address.trim().parse().ok()?);
        let max = if network.is_ipv4() { 32 } else { 128 };
        let prefix = prefix.unwrap_or(max);
        (prefix <= max).then_some(Self { network, prefix })
    }

    pub(crate) fn contains(&self, ip: IpAddr) -> bool {
        match (self.network, canonical(ip)) {
            (IpAddr::V4(network), IpAddr::V4(ip)) => {
                let mask = u32::MAX
                    .checked_shl(32 - u32::from(self.prefix))
                    .unwrap_or(0);
                u32::from(network) & mask == u32::from(ip) & mask
            }
            (IpAddr::V6(network), IpAddr::V6(ip)) => {
                let mask = u128::MAX
                    .checked_shl(128 - u32::from(self.prefix))
                    .unwrap_or(0);
                u128::from(network) & mask == u128::from(ip) & mask
            }
            _ => false,
        }
    }
}

/// An IPv4 peer reached over a dual-stack socket arrives as `::ffff:a.b.c.d`.
fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        IpAddr::V4(_) => ip,
    }
}

pub(crate) fn parse_list(value: &str) -> Result<Vec<Cidr>, String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            Cidr::parse(entry).ok_or_else(|| format!("invalid TRUSTED_PROXIES entry {entry:?}"))
        })
        .collect()
}

/// Reads `TRUSTED_PROXIES` once at startup; unset or empty keeps the default
/// of loopback and private networks.
pub fn configure(value: Option<&str>) -> Result<(), String> {
    let value = value
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(DEFAULT_TRUSTED);
    let _ = TRUSTED.set(parse_list(value)?);
    Ok(())
}

fn trusted() -> &'static [Cidr] {
    TRUSTED.get_or_init(|| parse_list(DEFAULT_TRUSTED).expect("default trusted proxies"))
}

/// The client address: the right-most forwarded address that is not one of
/// the trusted proxies, read only when the TCP peer itself is trusted.
pub(crate) fn client_address(
    peer: IpAddr,
    forwarded_for: Option<&str>,
    trusted: &[Cidr],
) -> IpAddr {
    let is_trusted = |ip: IpAddr| trusted.iter().any(|cidr| cidr.contains(ip));
    if !is_trusted(peer) {
        return canonical(peer);
    }
    let mut client = canonical(peer);
    for entry in forwarded_for.unwrap_or_default().rsplit(',') {
        let Ok(ip) = entry.trim().parse::<IpAddr>() else {
            break;
        };
        client = canonical(ip);
        if !is_trusted(client) {
            break;
        }
    }
    client
}

/// Replaces the forwarding headers with the one address `auth::client_ip`
/// reads, and drops the scheme a client that is not a trusted proxy claims.
pub async fn middleware(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    mut request: Request,
    next: Next,
) -> Response {
    let headers = request.headers_mut();
    let forwarded_for = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect::<Vec<_>>()
        .join(",");
    let trusted = trusted();
    if !trusted.iter().any(|cidr| cidr.contains(peer.ip())) {
        headers.remove("x-forwarded-proto");
        headers.remove("x-forwarded-host");
    }
    let client = client_address(peer.ip(), Some(&forwarded_for), trusted);
    headers.remove("x-real-ip");
    if let Ok(value) = HeaderValue::from_str(&client.to_string()) {
        headers.insert("x-forwarded-for", value);
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(value: &str) -> IpAddr {
        value.parse().unwrap()
    }

    #[test]
    fn cidrs_match_their_networks_only() {
        let list = parse_list(DEFAULT_TRUSTED).unwrap();
        let trusted = |value| list.iter().any(|cidr| cidr.contains(ip(value)));
        assert!(trusted("127.0.0.1"));
        assert!(trusted("::ffff:10.1.2.3"));
        assert!(trusted("172.31.255.255"));
        assert!(trusted("fd12::1"));
        assert!(!trusted("172.32.0.1"));
        assert!(!trusted("203.0.113.10"));
        assert!(!trusted("2001:db8::1"));
        assert_eq!(
            Cidr::parse("0.0.0.0/0").map(|c| c.contains(ip("8.8.8.8"))),
            Some(true)
        );
        assert!(parse_list("10.0.0.0/33").is_err());
        assert!(parse_list("example.com").is_err());
    }

    #[test]
    fn forwarded_addresses_count_only_from_trusted_proxies() {
        let trusted = parse_list(DEFAULT_TRUSTED).unwrap();
        // A client on the internet cannot pick its own address.
        assert_eq!(
            client_address(ip("198.51.100.7"), Some("1.2.3.4"), &trusted),
            ip("198.51.100.7")
        );
        // Behind a local proxy the address it appended is used, not one the
        // client prepended.
        assert_eq!(
            client_address(ip("127.0.0.1"), Some("1.2.3.4, 198.51.100.7"), &trusted),
            ip("198.51.100.7")
        );
        // Chained trusted proxies are skipped.
        assert_eq!(
            client_address(ip("10.0.0.2"), Some("198.51.100.7, 10.0.0.9"), &trusted),
            ip("198.51.100.7")
        );
        assert_eq!(
            client_address(ip("127.0.0.1"), None, &trusted),
            ip("127.0.0.1")
        );
        assert_eq!(
            client_address(ip("127.0.0.1"), Some("garbage"), &trusted),
            ip("127.0.0.1")
        );
    }
}
