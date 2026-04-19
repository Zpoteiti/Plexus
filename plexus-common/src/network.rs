//! SSRF-safe URL validation.
//!
//! Ported from `nanobot/security/network.py`. `validate_url` parses a URL,
//! checks the scheme, resolves the host to IPs, and rejects any IP that
//! falls in a blocked CIDR range unless a per-caller whitelist punches a
//! hole for it.
//!
//! Two configurations share this module:
//! - Server `web_fetch` passes an empty whitelist (unconditional RFC-1918
//!   block).
//! - Client-side network-touching tools pass the device's `ssrf_whitelist`
//!   to allow e.g. Tailscale (`100.64.0.0/10`) or a LAN range.

use ipnet::IpNet;
use std::net::{IpAddr, ToSocketAddrs};
use std::str::FromStr;
use url::Url;

use crate::errors::network::NetworkError;

/// RFC-1918 + link-local + loopback + carrier-grade NAT + IPv6 private
/// ranges. Ported from `nanobot/security/network.py::_BLOCKED_NETWORKS`.
pub fn blocked_networks() -> Vec<IpNet> {
    [
        "0.0.0.0/8",
        "10.0.0.0/8",
        "100.64.0.0/10", // carrier-grade NAT
        "127.0.0.0/8",
        "169.254.0.0/16", // link-local / cloud metadata
        "172.16.0.0/12",
        "192.168.0.0/16",
        "::1/128",
        "fc00::/7",  // IPv6 unique local
        "fe80::/10", // IPv6 link-local
    ]
    .iter()
    .map(|s| IpNet::from_str(s).expect("hardcoded CIDR"))
    .collect()
}

fn is_allowed(ip: IpAddr, whitelist: &[IpNet]) -> bool {
    if whitelist.iter().any(|n| n.contains(&ip)) {
        return true;
    }
    !blocked_networks().iter().any(|n| n.contains(&ip))
}

/// Parse `url`, verify scheme is http/https, resolve host, and reject if
/// any resolved IP is in a blocked network and not in `whitelist`.
#[must_use = "URL validation result must be checked"]
pub fn validate_url(url: &str, whitelist: &[IpNet]) -> Result<(), NetworkError> {
    let parsed = Url::parse(url).map_err(|_| NetworkError::InvalidUrl)?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(NetworkError::InvalidScheme);
    }
    let host = parsed.host_str().ok_or(NetworkError::MissingHost)?;
    let port = parsed.port_or_known_default().unwrap_or(80);
    let ips: Vec<IpAddr> = (host, port)
        .to_socket_addrs()
        .map_err(|_| NetworkError::ResolutionFailed)?
        .map(|sa| sa.ip())
        .collect();
    for ip in ips {
        if !is_allowed(ip, whitelist) {
            return Err(NetworkError::BlockedNetwork(ip));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_rfc1918() {
        assert!(validate_url("http://10.0.0.1/foo", &[]).is_err());
        assert!(validate_url("http://192.168.1.1/", &[]).is_err());
        assert!(validate_url("http://172.16.0.1/", &[]).is_err());
    }

    #[test]
    fn blocks_metadata_endpoint() {
        assert!(validate_url("http://169.254.169.254/meta", &[]).is_err());
    }

    #[test]
    fn allows_public() {
        assert!(validate_url("http://8.8.8.8/", &[]).is_ok());
    }

    #[test]
    fn whitelist_punches_hole() {
        let wl = vec![IpNet::from_str("10.180.0.0/16").unwrap()];
        assert!(validate_url("http://10.180.1.1/", &wl).is_ok());
        assert!(validate_url("http://10.0.0.1/", &wl).is_err()); // not in whitelist
    }

    #[test]
    fn rejects_non_http_schemes() {
        assert!(validate_url("file:///etc/passwd", &[]).is_err());
        assert!(validate_url("ftp://example.com/", &[]).is_err());
    }

    #[test]
    fn blocked_networks_list_matches_nanobot() {
        // Spot-check a few CIDRs
        let bn = blocked_networks();
        assert!(bn.iter().any(|n| n.to_string() == "10.0.0.0/8"));
        assert!(bn.iter().any(|n| n.to_string() == "169.254.0.0/16"));
        assert!(bn.iter().any(|n| n.to_string() == "::1/128"));
    }
}
