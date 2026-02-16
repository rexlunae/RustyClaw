//! TLS/WSS support for the gateway
//!
//! Provides:
//! - Certificate loading from PEM files
//! - Self-signed certificate generation for development
//! - TLS acceptor for secure WebSocket connections

use anyhow::{Context, Result};
use tokio_rustls::rustls::ServerConfig;
use rustls_pemfile::{certs, private_key};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio_rustls::TlsAcceptor;

/// Load TLS server configuration from certificate and key files
pub fn load_tls_config(cert_path: &Path, key_path: &Path) -> Result<Arc<ServerConfig>> {
    // Load certificate chain
    let cert_file = File::open(cert_path)
        .with_context(|| format!("Failed to open certificate file: {:?}", cert_path))?;
    let mut cert_reader = BufReader::new(cert_file);
    let cert_chain: Vec<_> = certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to parse certificate chain")?;

    if cert_chain.is_empty() {
        anyhow::bail!("No certificates found in {}", cert_path.display());
    }

    // Load private key
    let key_file = File::open(key_path)
        .with_context(|| format!("Failed to open private key file: {:?}", key_path))?;
    let mut key_reader = BufReader::new(key_file);
    let key = private_key(&mut key_reader)
        .context("Failed to parse private key")?
        .ok_or_else(|| anyhow::anyhow!("No private key found in {}", key_path.display()))?;

    // Build TLS configuration
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)
        .context("Failed to build TLS configuration")?;

    Ok(Arc::new(config))
}

/// Generate a self-signed certificate for development/local use
pub fn generate_self_signed_cert() -> Result<(Vec<u8>, Vec<u8>)> {
    use rcgen::{CertificateParams, DistinguishedName, KeyPair};

    eprintln!("[TLS] Generating self-signed certificate for development...");

    // Create certificate parameters
    let mut params = CertificateParams::default();
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "RustyClaw Gateway");
    params
        .distinguished_name
        .push(rcgen::DnType::OrganizationName, "RustyClaw");

    // Add Subject Alternative Names (SANs) for localhost
    params.subject_alt_names = vec![
        rcgen::SanType::DnsName(rcgen::Ia5String::try_from("localhost").unwrap()),
        rcgen::SanType::IpAddress(std::net::IpAddr::V4(std::net::Ipv4Addr::new(
            127, 0, 0, 1,
        ))),
        rcgen::SanType::IpAddress(std::net::IpAddr::V6(std::net::Ipv6Addr::new(
            0, 0, 0, 0, 0, 0, 0, 1,
        ))),
    ];

    // Set validity period (1 year)
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now;
    params.not_after = now + time::Duration::days(365);

    // Generate key pair and certificate
    let key_pair = KeyPair::generate().context("Failed to generate key pair")?;
    let cert = params
        .self_signed(&key_pair)
        .context("Failed to generate self-signed certificate")?;

    let cert_pem = cert.pem().into_bytes();
    let key_pem = key_pair.serialize_pem().into_bytes();

    eprintln!("[TLS] Self-signed certificate generated successfully");
    eprintln!("[TLS] Certificate is valid for localhost and 127.0.0.1");
    eprintln!("[TLS] WARNING: This certificate is not trusted by browsers/clients");
    eprintln!("[TLS] For production, use a certificate from a trusted CA (e.g., Let's Encrypt)");

    Ok((cert_pem, key_pem))
}

/// Create TLS acceptor from configuration
pub async fn create_tls_acceptor(
    cert_path: Option<&Path>,
    key_path: Option<&Path>,
    self_signed: bool,
) -> Result<TlsAcceptor> {
    let config = if self_signed {
        // Generate self-signed certificate
        let (cert_pem, key_pem) = generate_self_signed_cert()?;

        // Parse generated certificate
        let cert_chain: Vec<_> = certs(&mut cert_pem.as_slice())
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to parse generated certificate")?;

        let key = private_key(&mut key_pem.as_slice())
            .context("Failed to parse generated private key")?
            .ok_or_else(|| anyhow::anyhow!("No private key in generated data"))?;

        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(cert_chain, key)
            .context("Failed to build TLS configuration from generated certificate")?
    } else {
        // Load from files
        let cert_path = cert_path.ok_or_else(|| anyhow::anyhow!("TLS enabled but no cert_path specified"))?;
        let key_path = key_path.ok_or_else(|| anyhow::anyhow!("TLS enabled but no key_path specified"))?;

        load_tls_config(cert_path, key_path)?.as_ref().clone()
    };

    Ok(TlsAcceptor::from(Arc::new(config)))
}

/// Accept a TLS connection from a TCP stream
pub async fn accept_tls(
    acceptor: &TlsAcceptor,
    stream: TcpStream,
) -> Result<tokio_rustls::server::TlsStream<TcpStream>> {
    acceptor
        .accept(stream)
        .await
        .context("TLS handshake failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_self_signed_cert() {
        let result = generate_self_signed_cert();
        assert!(result.is_ok(), "Failed to generate self-signed cert: {:?}", result.err());

        let (cert_pem, key_pem) = result.unwrap();
        assert!(!cert_pem.is_empty(), "Certificate PEM is empty");
        assert!(!key_pem.is_empty(), "Private key PEM is empty");

        // Verify the PEM format
        assert!(cert_pem.starts_with(b"-----BEGIN CERTIFICATE-----"));
        assert!(key_pem.starts_with(b"-----BEGIN PRIVATE KEY-----"));
    }
}
