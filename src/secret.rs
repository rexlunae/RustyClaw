use zeroize::Zeroizing;

/// Small secret wrapper with redacted debug output and automatic zeroization.
#[derive(Default)]
pub struct SecretString(Zeroizing<String>);

impl SecretString {
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }
}

impl Clone for SecretString {
    fn clone(&self) -> Self {
        Self::new(self.0.to_string())
    }
}

impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}

pub trait ExposeSecret {
    fn expose_secret(&self) -> &str;
}

impl ExposeSecret for SecretString {
    fn expose_secret(&self) -> &str {
        self.0.as_str()
    }
}
