//! SSRF (Server-Side Request Forgery) protection
//!
//! Validates URLs before making HTTP requests to prevent:
//! - Access to private IP ranges
//! - Access to localhost
//! - Access to cloud metadata endpoints
//! - DNS rebinding attacks
//! - Unicode homograph attacks in domains

use ipnetwork::IpNetwork;
use std::net::{IpAddr, ToSocketAddrs};
use std::str::FromStr;
use tracing::warn;

/// Errors produced by [`SsrfValidator`].
///
/// Security-sensitive callers can distinguish a hard policy rejection
/// ([`SsrfError::Blocked`], [`SsrfError::NonAsciiHost`]) from an
/// environmental failure such as [`SsrfError::Resolution`].
#[derive(Debug, thiserror::Error)]
pub enum SsrfError {
    /// The URL could not be parsed.
    #[error("Invalid URL: {0}")]
    InvalidUrl(#[from] url::ParseError),
    /// A custom blocked range was not valid CIDR notation.
    #[error("Invalid CIDR notation: {0}")]
    InvalidCidr(#[from] ipnetwork::IpNetworkError),
    /// The URL scheme is not http/https.
    #[error("Invalid URL scheme '{0}': only http:// and https:// are allowed")]
    InvalidScheme(String),
    /// The URL has no host component.
    #[error("URL has no host")]
    NoHost,
    /// The host contains non-ASCII characters (potential homograph attack).
    #[error("Security: Domain contains non-ASCII characters (potential homograph attack): {0}")]
    NonAsciiHost(String),
    /// DNS resolution failed (initial lookup).
    #[error("Failed to resolve hostname '{host}': {source}")]
    Resolution {
        host: String,
        #[source]
        source: std::io::Error,
    },
    /// DNS resolution failed (rebinding re-check).
    #[error("DNS recheck failed for '{host}': {source}")]
    ResolutionRecheck {
        host: String,
        #[source]
        source: std::io::Error,
    },
    /// The hostname resolved to no IP addresses.
    #[error("Hostname '{0}' resolved to no IP addresses")]
    NoAddresses(String),
    /// A resolved IP falls inside a blocked range.
    #[error("Security: Access to {ip} is blocked (matches blocked range {range})")]
    Blocked { ip: IpAddr, range: IpNetwork },
}

/// SSRF validator with configurable blocked CIDR ranges
#[derive(Debug, Clone)]
pub struct SsrfValidator {
    /// List of blocked IP ranges (CIDR notation)
    blocked_ranges: Vec<IpNetwork>,
    /// Whether to allow private IPs (override for trusted environments)
    #[allow(dead_code)]
    allow_private_ips: bool,
}

impl Default for SsrfValidator {
    fn default() -> Self {
        Self::new(false)
    }
}

impl SsrfValidator {
    /// Create a new SSRF validator with default blocked ranges
    pub fn new(allow_private_ips: bool) -> Self {
        let blocked_ranges = if allow_private_ips {
            vec![
                // Only block cloud metadata endpoints if private IPs are allowed
                IpNetwork::from_str("169.254.169.254/32").unwrap(), // AWS/GCP/Azure metadata
            ]
        } else {
            vec![
                // Private IP ranges (RFC 1918)
                IpNetwork::from_str("10.0.0.0/8").unwrap(),
                IpNetwork::from_str("172.16.0.0/12").unwrap(),
                IpNetwork::from_str("192.168.0.0/16").unwrap(),
                // Localhost
                IpNetwork::from_str("127.0.0.0/8").unwrap(),
                IpNetwork::from_str("::1/128").unwrap(),
                // Link-local
                IpNetwork::from_str("169.254.0.0/16").unwrap(),
                IpNetwork::from_str("fe80::/10").unwrap(),
                // Unique-local, fc00::/7. This is the IPv6 equivalent of
                // 10.0.0.0/8 and is what IPv6 private networking actually
                // uses, and it was absent: `https://[fd00::1]/` passed every
                // check and connected (issue #432).
                IpNetwork::from_str("fc00::/7").unwrap(),
                // Unspecified. `::` reaches loopback on Linux the same way
                // `0.0.0.0` does.
                IpNetwork::from_str("::/128").unwrap(),
                // Discard-only, RFC 6666.
                IpNetwork::from_str("100::/64").unwrap(),
                // Documentation, and multicast — the IPv6 halves of the
                // 198.51.100.0/24 and 224.0.0.0/4 entries below, which had
                // no counterpart.
                IpNetwork::from_str("2001:db8::/32").unwrap(),
                IpNetwork::from_str("ff00::/8").unwrap(),
                // Other reserved ranges
                IpNetwork::from_str("0.0.0.0/8").unwrap(),
                IpNetwork::from_str("100.64.0.0/10").unwrap(), // Carrier-grade NAT
                IpNetwork::from_str("192.0.0.0/24").unwrap(),  // IETF protocol assignments
                IpNetwork::from_str("192.0.2.0/24").unwrap(),  // TEST-NET-1
                IpNetwork::from_str("198.18.0.0/15").unwrap(), // Benchmarking
                IpNetwork::from_str("198.51.100.0/24").unwrap(), // TEST-NET-2
                IpNetwork::from_str("203.0.113.0/24").unwrap(), // TEST-NET-3
                IpNetwork::from_str("224.0.0.0/4").unwrap(),   // Multicast
                IpNetwork::from_str("240.0.0.0/4").unwrap(),   // Reserved
                IpNetwork::from_str("255.255.255.255/32").unwrap(), // Broadcast
            ]
        };

        Self {
            blocked_ranges,
            allow_private_ips,
        }
    }

    /// Add a custom blocked CIDR range
    pub fn add_blocked_range(&mut self, cidr: &str) -> Result<(), SsrfError> {
        let network = IpNetwork::from_str(cidr)?;
        self.blocked_ranges.push(network);
        Ok(())
    }

    /// Validate a URL for SSRF vulnerabilities
    pub fn validate_url(&self, url: &str) -> Result<(), SsrfError> {
        // Parse the URL
        let parsed_url = url::Url::parse(url)?;

        // 1. Validate scheme (only http/https allowed)
        let scheme = parsed_url.scheme();
        if scheme != "http" && scheme != "https" {
            return Err(SsrfError::InvalidScheme(scheme.to_string()));
        }

        // 2. Get the host
        let host = parsed_url.host_str().ok_or(SsrfError::NoHost)?;

        // 3. Check for Unicode homograph attacks (non-ASCII characters in domain)
        if !host.is_ascii() {
            return Err(SsrfError::NonAsciiHost(host.to_string()));
        }

        // 4. Resolve hostname to IP addresses
        let socket_addr_str = if let Some(port) = parsed_url.port() {
            format!("{}:{}", host, port)
        } else {
            // Use default ports for scheme
            let default_port = if scheme == "https" { 443 } else { 80 };
            format!("{}:{}", host, default_port)
        };

        let ip_addrs: Vec<IpAddr> = socket_addr_str
            .to_socket_addrs()
            .map_err(|e| SsrfError::Resolution {
                host: host.to_string(),
                source: e,
            })?
            .map(|sa| sa.ip())
            .collect();

        if ip_addrs.is_empty() {
            return Err(SsrfError::NoAddresses(host.to_string()));
        }

        // 5. Check all resolved IPs against blocked ranges
        for ip in &ip_addrs {
            self.validate_ip(ip)?;
        }

        // 6. DNS rebinding protection: resolve again and verify IPs haven't changed
        // This helps detect time-of-check-time-of-use attacks
        let recheck_ips: Vec<IpAddr> = socket_addr_str
            .to_socket_addrs()
            .map_err(|e| SsrfError::ResolutionRecheck {
                host: host.to_string(),
                source: e,
            })?
            .map(|sa| sa.ip())
            .collect();

        // Verify that all IPs from both resolutions are safe
        for ip in &recheck_ips {
            self.validate_ip(ip)?;
        }

        // Check if IP sets differ (potential DNS rebinding)
        if ip_addrs.len() != recheck_ips.len()
            || !ip_addrs.iter().all(|ip| recheck_ips.contains(ip))
        {
            warn!(
                host = %host,
                initial_ips = ?ip_addrs,
                recheck_ips = ?recheck_ips,
                "DNS resolution changed between checks — possible DNS rebinding"
            );
            // Allow it but log the warning - legitimate round-robin DNS can cause this
        }

        Ok(())
    }

    /// Validate a single IP address against blocked ranges.
    ///
    /// Public so that other guards in this crate check the same list rather
    /// than keeping a second copy — the ClawHub upload guard grew one, and the
    /// two had already drifted in both directions by the time anyone compared
    /// them (issue #432).
    pub fn validate_ip(&self, ip: &IpAddr) -> Result<(), SsrfError> {
        // An IPv4 address wearing an IPv6 hat is still that address, and
        // `::ffff:10.0.0.1` is not inside any of the v6 ranges above. The
        // list used to carry `::ffff:127.0.0.0/104` — the mapped loopback
        // prefix and nothing else — so mapped RFC1918 and mapped link-local
        // walked through. Unwrapping covers every v4 rule at once, including
        // any added later, rather than adding three more prefixes that then
        // have to be kept in step.
        //
        // `to_ipv4` also unwraps the deprecated IPv4-compatible form
        // (`::a.b.c.d`). That is deliberate: those are not real destinations,
        // and the ones it would newly block all land in `0.0.0.0/8`.
        if let IpAddr::V6(v6) = ip
            && let Some(v4) = v6.to_ipv4()
        {
            self.check_against_ranges(&IpAddr::V4(v4))?;
        }
        self.check_against_ranges(ip)
    }

    /// The literal range comparison, without the IPv4-mapped unwrapping.
    fn check_against_ranges(&self, ip: &IpAddr) -> Result<(), SsrfError> {
        for blocked_range in &self.blocked_ranges {
            if blocked_range.contains(*ip) {
                return Err(SsrfError::Blocked {
                    ip: *ip,
                    range: *blocked_range,
                });
            }
        }
        Ok(())
    }

    /// Check if a URL would be blocked (non-failing version for testing)
    pub fn is_blocked(&self, url: &str) -> bool {
        self.validate_url(url).is_err()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blocks_private_ips() {
        let validator = SsrfValidator::new(false);

        // Private IPs should be blocked
        assert!(validator.is_blocked("http://192.168.1.1/"));
        assert!(validator.is_blocked("http://10.0.0.1/"));
        assert!(validator.is_blocked("http://172.16.0.1/"));
    }

    #[test]
    fn test_blocks_localhost() {
        let validator = SsrfValidator::new(false);

        assert!(validator.is_blocked("http://127.0.0.1/"));
        assert!(validator.is_blocked("http://localhost/"));
    }

    #[test]
    fn test_blocks_cloud_metadata() {
        let validator = SsrfValidator::new(false);

        assert!(validator.is_blocked("http://169.254.169.254/latest/meta-data/"));
    }

    #[test]
    fn test_blocks_invalid_schemes() {
        let validator = SsrfValidator::new(false);

        assert!(validator.is_blocked("file:///etc/passwd"));
        assert!(validator.is_blocked("ftp://example.com/"));
        assert!(validator.is_blocked("javascript:alert(1)"));
    }

    #[test]
    fn test_allows_public_urls() {
        let validator = SsrfValidator::new(false);

        // These should succeed (though DNS resolution might fail in tests)
        // We're just testing the validation logic, not actual network access
        let result = validator.validate_url("https://example.com/");
        // May fail due to DNS in test environment, but shouldn't fail for SSRF reasons
        if let Err(e) = result {
            assert!(
                !matches!(e, SsrfError::Blocked { .. } | SsrfError::NonAsciiHost(_)),
                "Should not be a security error: {e}"
            );
        }
    }

    #[test]
    fn test_allow_private_ips_override() {
        let validator = SsrfValidator::new(true);

        // With allow_private_ips=true, private IPs should be allowed
        // (but metadata endpoints still blocked)
        // Note: This will fail DNS resolution in tests, but that's expected
        let result = validator.validate_url("http://192.168.1.1/");
        if let Err(e) = result {
            // Should fail DNS resolution, not security check
            assert!(
                !matches!(e, SsrfError::Blocked { .. }),
                "Should not be blocked: {e}"
            );
        }
    }

    #[test]
    fn test_custom_blocked_range() {
        let mut validator = SsrfValidator::new(false);
        validator.add_blocked_range("8.8.8.0/24").unwrap();

        // 8.8.8.8 should now be blocked
        assert!(validator.is_blocked("http://8.8.8.8/"));
    }
}

/// The address list, checked directly rather than through a URL.
///
/// `validate_url` resolves, so a test written against it needs DNS and is at
/// the mercy of whatever the resolver returns. These call `validate_ip` with
/// the address the resolver would have produced, which is the part the ranges
/// actually govern.
#[cfg(test)]
mod address_tests {
    use super::*;
    use std::net::IpAddr;
    use std::str::FromStr;

    fn blocked(addr: &str) -> bool {
        SsrfValidator::new(false)
            .validate_ip(&IpAddr::from_str(addr).expect("test address parses"))
            .is_err()
    }

    /// The gap in issue #432: `fd00::/8` is what IPv6 private networking
    /// actually uses, and the list carried only `::1` and `fe80::/10`, so
    /// `web_fetch(url="https://[fd00::1]/")` connected.
    #[test]
    fn ipv6_unique_local_is_blocked() {
        assert!(blocked("fd00::1"));
        assert!(blocked("fdff:1234:5678::5"));
        assert!(blocked("fc00::1"));
    }

    /// The mapped-address list was one prefix long — loopback — so an RFC1918
    /// or link-local address in IPv6 clothing was not in any blocked range.
    #[test]
    fn ipv4_addresses_in_ipv6_clothing_are_blocked() {
        for mapped in [
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::ffff:172.16.0.1",
            "::ffff:192.168.1.1",
            "::ffff:169.254.169.254",
            "::ffff:100.64.0.1",
        ] {
            assert!(blocked(mapped), "{mapped} should be blocked");
        }
    }

    /// The ranges that had an IPv4 entry and no IPv6 counterpart.
    #[test]
    fn the_ipv6_halves_of_the_existing_ipv4_rules_are_blocked() {
        assert!(blocked("::"), "unspecified");
        assert!(blocked("ff02::1"), "multicast");
        assert!(blocked("2001:db8::1"), "documentation");
        assert!(blocked("100::1"), "discard-only");
    }

    /// What was already covered, so a regression in the v4 half shows up here
    /// rather than only in the URL-level tests.
    #[test]
    fn the_ranges_that_already_worked_still_do() {
        for addr in [
            "127.0.0.1",
            "10.1.2.3",
            "192.168.0.5",
            "169.254.169.254",
            "::1",
            "fe80::1",
        ] {
            assert!(blocked(addr), "{addr} should be blocked");
        }
    }

    /// And routable addresses still go through — a rule that blocked
    /// everything would satisfy every test above.
    #[test]
    fn public_addresses_are_not_blocked() {
        for addr in ["93.184.216.34", "1.1.1.1", "2606:4700::1111", "2600::1"] {
            assert!(!blocked(addr), "{addr} should be allowed");
        }
    }

    /// `allow_private_ips` is an explicit opt-in for local development, and
    /// the metadata endpoint stays blocked even then. Checked because the
    /// mapped-address unwrapping runs in that mode too, and unwrapping
    /// against a one-entry list must not start blocking ordinary addresses.
    #[test]
    fn the_permissive_mode_still_blocks_the_metadata_endpoint() {
        let permissive = SsrfValidator::new(true);
        let metadata = IpAddr::from_str("169.254.169.254").unwrap();
        assert!(permissive.validate_ip(&metadata).is_err());
        assert!(
            permissive
                .validate_ip(&IpAddr::from_str("::ffff:169.254.169.254").unwrap())
                .is_err(),
            "the mapped form of the metadata endpoint is the same endpoint"
        );
        assert!(
            permissive
                .validate_ip(&IpAddr::from_str("10.0.0.1").unwrap())
                .is_ok(),
            "private addresses are the point of this mode"
        );
    }
}
