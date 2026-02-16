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

/// SSRF validator with configurable blocked CIDR ranges
#[derive(Debug, Clone)]
pub struct SsrfValidator {
    /// List of blocked IP ranges (CIDR notation)
    blocked_ranges: Vec<IpNetwork>,
    /// Whether to allow private IPs (override for trusted environments)
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
                // Loopback
                IpNetwork::from_str("::ffff:127.0.0.0/104").unwrap(), // IPv4-mapped IPv6 loopback
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
    pub fn add_blocked_range(&mut self, cidr: &str) -> Result<(), String> {
        let network =
            IpNetwork::from_str(cidr).map_err(|e| format!("Invalid CIDR notation: {}", e))?;
        self.blocked_ranges.push(network);
        Ok(())
    }

    /// Validate a URL for SSRF vulnerabilities
    pub fn validate_url(&self, url: &str) -> Result<(), String> {
        // Parse the URL
        let parsed_url = url::Url::parse(url).map_err(|e| format!("Invalid URL: {}", e))?;

        // 1. Validate scheme (only http/https allowed)
        let scheme = parsed_url.scheme();
        if scheme != "http" && scheme != "https" {
            return Err(format!(
                "Invalid URL scheme '{}': only http:// and https:// are allowed",
                scheme
            ));
        }

        // 2. Get the host
        let host = parsed_url
            .host_str()
            .ok_or_else(|| "URL has no host".to_string())?;

        // 3. Check for Unicode homograph attacks (non-ASCII characters in domain)
        if !host.is_ascii() {
            return Err(format!(
                "Security: Domain contains non-ASCII characters (potential homograph attack): {}",
                host
            ));
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
            .map_err(|e| format!("Failed to resolve hostname '{}': {}", host, e))?
            .map(|sa| sa.ip())
            .collect();

        if ip_addrs.is_empty() {
            return Err(format!("Hostname '{}' resolved to no IP addresses", host));
        }

        // 5. Check all resolved IPs against blocked ranges
        for ip in &ip_addrs {
            self.validate_ip(ip)?;
        }

        // 6. DNS rebinding protection: resolve again and verify IPs haven't changed
        // This helps detect time-of-check-time-of-use attacks
        let recheck_ips: Vec<IpAddr> = socket_addr_str
            .to_socket_addrs()
            .map_err(|e| format!("DNS recheck failed for '{}': {}", host, e))?
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
            eprintln!(
                "[Security] Warning: DNS resolution changed between checks for '{}': {:?} -> {:?}",
                host, ip_addrs, recheck_ips
            );
            // Allow it but log the warning - legitimate round-robin DNS can cause this
        }

        Ok(())
    }

    /// Validate a single IP address against blocked ranges
    fn validate_ip(&self, ip: &IpAddr) -> Result<(), String> {
        for blocked_range in &self.blocked_ranges {
            if blocked_range.contains(*ip) {
                return Err(format!(
                    "Security: Access to {} is blocked (matches blocked range {})",
                    ip, blocked_range
                ));
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
            assert!(!e.contains("Security:"), "Should not be a security error");
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
            assert!(!e.contains("Security:") || e.contains("Failed to resolve"));
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
