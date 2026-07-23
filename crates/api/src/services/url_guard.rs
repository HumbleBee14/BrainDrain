//! SSRF protection for user-supplied URLs.
//!
//! Any URL that the server itself will fetch on behalf of a tenant (notification
//! webhooks, per-tenant LLM `api_base_url`) must be validated so a tenant cannot
//! point it at private/internal/metadata endpoints and use the response — or even
//! just the connect/timeout timing — to reach or fingerprint the internal network.

use std::net::IpAddr;

/// Returns `true` if the IP address is in a private, reserved, or internal range
/// that a tenant-supplied URL must never be allowed to reach.
pub fn is_private_ip(ip: &IpAddr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return true;
    }
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            octets[0] == 10
                || (octets[0] == 172 && (16..=31).contains(&octets[1]))
                || (octets[0] == 192 && octets[1] == 168)
                || (octets[0] == 169 && octets[1] == 254)
                // Carrier-grade NAT (RFC 6598) — routable to internal infra on
                // some networks, so treat as private.
                || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        }
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            (segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xffc0) == 0xfe80
                || v6.is_loopback()
                || segments[0..5] == [0, 0, 0, 0, 0]
                    && segments[5] == 0xffff
                    && is_private_ip(&IpAddr::V4(std::net::Ipv4Addr::new(
                        (segments[6] >> 8) as u8,
                        segments[6] as u8,
                        (segments[7] >> 8) as u8,
                        segments[7] as u8,
                    )))
        }
    }
}

/// Returns `true` if `url` is a plain http(s) URL that resolves only to public
/// addresses. Rejects non-http(s) schemes, unresolvable hosts, and any host that
/// resolves to a private/internal/metadata address (SSRF protection).
pub async fn is_safe_public_url(url: &str) -> bool {
    let parsed = match reqwest::Url::parse(url) {
        Ok(u) => u,
        Err(_) => return false,
    };

    if parsed.scheme() != "https" && parsed.scheme() != "http" {
        return false;
    }

    let host = match parsed.host_str() {
        Some(h) => h,
        None => return false,
    };

    let port = parsed
        .port()
        .unwrap_or(if parsed.scheme() == "https" { 443 } else { 80 });
    let addrs = match tokio::net::lookup_host(format!("{host}:{port}")).await {
        Ok(a) => a,
        Err(_) => return false,
    };

    let mut found_any = false;
    for addr in addrs {
        found_any = true;
        if is_private_ip(&addr.ip()) {
            return false;
        }
    }

    found_any
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_ips_detected() {
        assert!(is_private_ip(&"127.0.0.1".parse().unwrap()));
        assert!(is_private_ip(&"10.0.0.1".parse().unwrap()));
        assert!(is_private_ip(&"172.16.0.1".parse().unwrap()));
        assert!(is_private_ip(&"192.168.1.1".parse().unwrap()));
        assert!(is_private_ip(&"169.254.169.254".parse().unwrap()));
        assert!(is_private_ip(&"::1".parse().unwrap()));
    }

    #[test]
    fn cgnat_range_detected() {
        // RFC 6598 100.64.0.0/10
        assert!(is_private_ip(&"100.64.0.1".parse().unwrap()));
        assert!(is_private_ip(&"100.127.255.255".parse().unwrap()));
        // Just outside the /10 must still be public.
        assert!(!is_private_ip(&"100.63.255.255".parse().unwrap()));
        assert!(!is_private_ip(&"100.128.0.1".parse().unwrap()));
    }

    #[test]
    fn public_ips_allowed() {
        assert!(!is_private_ip(&"8.8.8.8".parse().unwrap()));
        assert!(!is_private_ip(&"1.1.1.1".parse().unwrap()));
        assert!(!is_private_ip(&"203.0.113.1".parse().unwrap()));
    }

    #[tokio::test]
    async fn rejects_non_http_scheme() {
        assert!(!is_safe_public_url("ftp://example.com").await);
        assert!(!is_safe_public_url("file:///etc/passwd").await);
    }

    #[tokio::test]
    async fn rejects_loopback_host() {
        assert!(!is_safe_public_url("http://127.0.0.1:6379").await);
        assert!(!is_safe_public_url("http://localhost/").await);
    }
}
