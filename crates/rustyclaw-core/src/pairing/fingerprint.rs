//! Key fingerprint utilities.

use super::client_keys::ClientKeyPair;

/// Calculate the SHA256 fingerprint of a public key.
///
/// Returns a string like "SHA256:AbCdEf...".
pub fn key_fingerprint(keypair: &ClientKeyPair) -> String {
    keypair
        .public_key
        .fingerprint(russh::keys::HashAlg::Sha256)
        .to_string()
}

/// Get a short fingerprint (last 8 characters of the hash).
///
/// Useful for display in limited space.
pub fn key_fingerprint_short(keypair: &ClientKeyPair) -> String {
    let fp = key_fingerprint(keypair);
    // Skip "SHA256:" prefix and take last 8 chars
    if fp.starts_with("SHA256:") && fp.len() > 15 {
        fp[fp.len() - 8..].to_string()
    } else {
        fp
    }
}

/// Calculate fingerprint from an OpenSSH public key string.
#[allow(dead_code)]
pub fn calculate_fingerprint_from_openssh(public_key_openssh: &str) -> String {
    use base64::Engine;
    use sha2::{Digest, Sha256};

    // Parse the key to get the base64 data
    let parts: Vec<&str> = public_key_openssh.split_whitespace().collect();
    if parts.len() < 2 {
        return "SHA256:invalid".to_string();
    }

    // Decode the base64 key data
    let key_data = match base64::engine::general_purpose::STANDARD.decode(parts[1]) {
        Ok(data) => data,
        Err(_) => return "SHA256:invalid".to_string(),
    };

    // Calculate SHA256 hash
    let mut hasher = Sha256::new();
    hasher.update(&key_data);
    let hash = hasher.finalize();

    // Encode as base64 (without padding, to match ssh-keygen format)
    let fingerprint = base64::engine::general_purpose::STANDARD_NO_PAD.encode(&hash);

    format!("SHA256:{}", fingerprint)
}

/// Generate ASCII art representation of a key fingerprint.
///
/// Similar to `ssh-keygen -lv`, this creates a visual hash that makes
/// it easier to verify keys by eye.
pub fn format_fingerprint_art(fingerprint: &str) -> String {
    use base64::Engine;

    // Extract the hash part (after "SHA256:")
    let hash = fingerprint.strip_prefix("SHA256:").unwrap_or(fingerprint);

    // Decode the base64 to get raw bytes
    let bytes = match base64::engine::general_purpose::STANDARD_NO_PAD.decode(hash) {
        Ok(b) => b,
        Err(_) => return format!("[{}]", fingerprint),
    };

    // Generate the randomart image (17x9 field)
    // This is the "drunken bishop" algorithm
    const FIELD_WIDTH: usize = 17;
    const FIELD_HEIGHT: usize = 9;
    let mut field = [[0u8; FIELD_WIDTH]; FIELD_HEIGHT];

    // Bishop starts in the center
    let mut x: i32 = (FIELD_WIDTH / 2) as i32;
    let mut y: i32 = (FIELD_HEIGHT / 2) as i32;

    // Walk the field based on hash bits
    for byte in &bytes {
        for i in 0..4 {
            let step = (byte >> (i * 2)) & 0x03;
            match step {
                0 => {
                    x -= 1;
                    y -= 1;
                } // upper left
                1 => {
                    x += 1;
                    y -= 1;
                } // upper right
                2 => {
                    x -= 1;
                    y += 1;
                } // lower left
                3 => {
                    x += 1;
                    y += 1;
                } // lower right
                _ => unreachable!(),
            }

            // Clamp to field boundaries
            x = x.clamp(0, (FIELD_WIDTH - 1) as i32);
            y = y.clamp(0, (FIELD_HEIGHT - 1) as i32);

            // Increment the cell visit count
            let cell = &mut field[y as usize][x as usize];
            if *cell < 14 {
                *cell += 1;
            }
        }
    }

    // Mark start and end positions
    field[FIELD_HEIGHT / 2][FIELD_WIDTH / 2] = 15; // 'S' for start
    field[y as usize][x as usize] = 16; // 'E' for end

    // Convert to characters
    const CHARS: &[char] = &[
        ' ', '.', 'o', '+', '=', '*', 'B', 'O', 'X', '@', '%', '&', '#', '/', '^', 'S', 'E',
    ];

    let mut output = String::new();
    output.push_str("+---[ED25519 256]----+\n");

    for row in &field {
        output.push('|');
        for &cell in row {
            let c = CHARS.get(cell as usize).copied().unwrap_or('?');
            output.push(c);
        }
        output.push_str("|\n");
    }

    output.push_str("+----[SHA256]--------+");

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fingerprint_from_openssh() {
        // This is a test key (not real)
        let key =
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIKtJvJZDLNbPkTYf4ZbXaBeCq3I9sEG9qS9XvGBFMT4C test";
        let fp = calculate_fingerprint_from_openssh(key);

        assert!(fp.starts_with("SHA256:"));
        assert!(fp.len() > 10);
    }

    #[test]
    fn test_fingerprint_art() {
        let key =
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIKtJvJZDLNbPkTYf4ZbXaBeCq3I9sEG9qS9XvGBFMT4C test";
        let fingerprint = calculate_fingerprint_from_openssh(key);
        let art = format_fingerprint_art(&fingerprint);

        assert!(art.contains("+---[ED25519"));
        assert!(art.contains("+----[SHA256]"));
        assert!(art.lines().count() == 11); // 9 rows + 2 borders
    }
}
