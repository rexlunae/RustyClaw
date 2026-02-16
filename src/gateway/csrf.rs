use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use rand::rngs::OsRng;
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub const DEFAULT_CSRF_TTL: Duration = Duration::from_secs(60 * 60);

/// In-memory CSRF token store with TTL expiry.
#[derive(Debug)]
pub struct CsrfStore {
    ttl: Duration,
    issued: HashMap<String, Instant>,
}

impl Default for CsrfStore {
    fn default() -> Self {
        Self::new(DEFAULT_CSRF_TTL)
    }
}

impl CsrfStore {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            issued: HashMap::new(),
        }
    }

    pub fn issue_token(&mut self) -> String {
        self.prune_expired();
        let token = generate_token();
        self.issued.insert(token.clone(), Instant::now());
        token
    }

    pub fn validate(&mut self, token: &str) -> bool {
        self.prune_expired();
        self.issued.contains_key(token)
    }

    fn prune_expired(&mut self) {
        let now = Instant::now();
        let ttl = self.ttl;
        self.issued.retain(|_, issued_at| now.duration_since(*issued_at) <= ttl);
    }
}

fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issued_token_is_32_random_bytes() {
        let mut store = CsrfStore::default();
        let token = store.issue_token();
        let decoded = URL_SAFE_NO_PAD.decode(token).unwrap();
        assert_eq!(decoded.len(), 32);
    }

    #[test]
    fn validates_fresh_token() {
        let mut store = CsrfStore::new(Duration::from_secs(60));
        let token = store.issue_token();
        assert!(store.validate(&token));
        assert!(!store.validate("not-a-real-token"));
    }

    #[test]
    fn rejects_expired_token() {
        let mut store = CsrfStore::new(Duration::from_millis(5));
        let token = store.issue_token();
        std::thread::sleep(Duration::from_millis(20));
        assert!(!store.validate(&token));
    }
}
