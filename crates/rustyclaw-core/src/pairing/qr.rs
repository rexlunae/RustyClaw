//! QR code generation and parsing for pairing.
//!
//! The QR code contains JSON-encoded pairing data:
//!
//! ```json
//! {
//!   "v": 1,
//!   "type": "client" | "gateway",
//!   "key": "ssh-ed25519 AAAA...",
//!   "name": "laptop@user",
//!   "host": "gateway.example.com:2222"  // only for gateway type
//! }
//! ```

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Pairing data encoded in QR codes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingData {
    /// Version number (currently 1).
    #[serde(rename = "v")]
    pub version: u8,
    
    /// Type of pairing data.
    #[serde(rename = "type")]
    pub pairing_type: PairingType,
    
    /// Public key in OpenSSH format.
    pub key: String,
    
    /// Human-readable name (e.g., "laptop@user").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    
    /// Gateway host:port (only for gateway pairing data).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
}

/// Type of pairing data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PairingType {
    /// Client offering its public key to a gateway.
    Client,
    /// Gateway offering its host info and key to a client.
    Gateway,
}

impl PairingData {
    /// Create client pairing data.
    pub fn client(public_key: &str, name: Option<String>) -> Self {
        Self {
            version: 1,
            pairing_type: PairingType::Client,
            key: public_key.to_string(),
            name,
            host: None,
        }
    }
    
    /// Create gateway pairing data.
    pub fn gateway(public_key: &str, host: &str, name: Option<String>) -> Self {
        Self {
            version: 1,
            pairing_type: PairingType::Gateway,
            key: public_key.to_string(),
            name,
            host: Some(host.to_string()),
        }
    }
    
    /// Encode to JSON.
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string(self).context("Failed to serialize pairing data")
    }
    
    /// Decode from JSON.
    pub fn from_json(json: &str) -> Result<Self> {
        serde_json::from_str(json).context("Failed to parse pairing data")
    }
}

/// Generate a QR code image for pairing.
///
/// Returns the QR code as a PNG image in bytes.
#[cfg(feature = "qr")]
pub fn generate_pairing_qr(data: &PairingData) -> Result<Vec<u8>> {
    use qrcode::QrCode;
    use image::{Luma, ImageEncoder, codecs::png::PngEncoder};
    
    let json = data.to_json()?;
    
    let code = QrCode::new(json.as_bytes())
        .context("Failed to generate QR code")?;
    
    // Render to image
    let image = code.render::<Luma<u8>>()
        .min_dimensions(200, 200)
        .max_dimensions(400, 400)
        .build();
    
    // Encode as PNG
    let mut png_data = Vec::new();
    let encoder = PngEncoder::new(&mut png_data);
    encoder.write_image(
        image.as_raw(),
        image.width(),
        image.height(),
        image::ExtendedColorType::L8,
    ).context("Failed to encode QR code as PNG")?;
    
    Ok(png_data)
}

/// Generate a QR code as ASCII art (for terminal display).
#[cfg(feature = "qr")]
pub fn generate_pairing_qr_ascii(data: &PairingData) -> Result<String> {
    use qrcode::QrCode;
    
    let json = data.to_json()?;
    
    let code = QrCode::new(json.as_bytes())
        .context("Failed to generate QR code")?;
    
    // Render to ASCII using Unicode block characters
    let mut output = String::new();
    let modules = code.to_colors();
    let width = code.width();
    
    // Use half-block characters for denser output
    for y in (0..width).step_by(2) {
        for x in 0..width {
            let top = modules[y * width + x] == qrcode::Color::Dark;
            let bottom = if y + 1 < width {
                modules[(y + 1) * width + x] == qrcode::Color::Dark
            } else {
                false
            };
            
            let ch = match (top, bottom) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            };
            output.push(ch);
        }
        output.push('\n');
    }
    
    Ok(output)
}

#[cfg(not(feature = "qr"))]
pub fn generate_pairing_qr(_data: &PairingData) -> Result<Vec<u8>> {
    anyhow::bail!("QR code feature not enabled")
}

#[cfg(not(feature = "qr"))]
pub fn generate_pairing_qr_ascii(_data: &PairingData) -> Result<String> {
    anyhow::bail!("QR code feature not enabled")
}

/// Parse pairing data from a QR code or JSON string.
pub fn parse_pairing_qr(input: &str) -> Result<PairingData> {
    // Try to parse as JSON directly
    if let Ok(data) = PairingData::from_json(input) {
        return Ok(data);
    }
    
    // Try to parse as an OpenSSH public key (raw key paste)
    if input.starts_with("ssh-") {
        return Ok(PairingData::client(input.trim(), None));
    }
    
    anyhow::bail!("Could not parse pairing data")
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_client_pairing_data() {
        let data = PairingData::client(
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5...",
            Some("test@laptop".to_string()),
        );
        
        assert_eq!(data.version, 1);
        assert_eq!(data.pairing_type, PairingType::Client);
        assert!(data.host.is_none());
        
        let json = data.to_json().unwrap();
        let parsed = PairingData::from_json(&json).unwrap();
        assert_eq!(parsed.key, data.key);
    }
    
    #[test]
    fn test_gateway_pairing_data() {
        let data = PairingData::gateway(
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5...",
            "gateway.example.com:2222",
            Some("my-gateway".to_string()),
        );
        
        assert_eq!(data.pairing_type, PairingType::Gateway);
        assert_eq!(data.host, Some("gateway.example.com:2222".to_string()));
    }
    
    #[test]
    fn test_parse_raw_key() {
        let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5... user@host";
        let data = parse_pairing_qr(key).unwrap();
        
        assert_eq!(data.pairing_type, PairingType::Client);
        assert_eq!(data.key, key);
    }
}
