//! WebAuthn/Passkey Authentication
//!
//! Provides modern passwordless authentication using WebAuthn passkeys.
//! Maintains TOTP as fallback authentication method.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use webauthn_rs::prelude::*;

/// WebAuthn authenticator for the gateway
pub struct WebAuthnAuth {
    webauthn: Webauthn,
    /// Active registration challenges (user_id -> registration_state)
    registration_challenges: Arc<Mutex<HashMap<String, PasskeyRegistration>>>,
    /// Active authentication challenges (user_id -> authentication_state)
    auth_challenges: Arc<Mutex<HashMap<String, PasskeyAuthentication>>>,
}

/// Stored passkey credential
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredPasskey {
    pub credential: Passkey,
    pub user_name: String,
    pub created_at: i64, // Unix timestamp
    pub last_used: Option<i64>,
}

impl WebAuthnAuth {
    /// Create a new WebAuthn authenticator
    ///
    /// # Arguments
    /// * `rp_id` - Relying Party ID (usually the domain, e.g., "localhost" or "example.com")
    /// * `rp_origin` - Relying Party origin (full URL, e.g., "https://localhost:8443")
    pub fn new(rp_id: &str, rp_origin: &str) -> Result<Self> {
        let rp_origin_parsed = Url::parse(rp_origin)
            .context("Failed to parse WebAuthn origin URL")?;

        let builder = WebauthnBuilder::new(rp_id, &rp_origin_parsed)
            .context("Failed to create WebAuthn builder")?;

        let webauthn = builder
            .rp_name("RustyClaw")
            .build()
            .context("Failed to build WebAuthn instance")?;

        Ok(Self {
            webauthn,
            registration_challenges: Arc::new(Mutex::new(HashMap::new())),
            auth_challenges: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Start passkey registration for a user
    ///
    /// Returns a challenge that should be sent to the client for registration.
    pub async fn start_registration(
        &self,
        user_id: &str,
        user_name: &str,
        existing_credentials: Vec<Passkey>,
    ) -> Result<CreationChallengeResponse> {
        let user_unique_id = Uuid::parse_str(user_id)
            .unwrap_or_else(|_| Uuid::new_v4());

        // Extract credential IDs from existing passkeys for exclusion
        let exclude_credentials = if existing_credentials.is_empty() {
            None
        } else {
            Some(
                existing_credentials
                    .iter()
                    .map(|pk| pk.cred_id().clone())
                    .collect()
            )
        };

        let (ccr, reg_state) = self.webauthn
            .start_passkey_registration(
                user_unique_id,
                user_name,
                user_name,
                exclude_credentials,
            )
            .context("Failed to start passkey registration")?;

        // Store registration state
        let mut challenges = self.registration_challenges.lock().await;
        challenges.insert(user_id.to_string(), reg_state);

        Ok(ccr)
    }

    /// Complete passkey registration
    ///
    /// Verifies the client's registration response and returns the credential.
    pub async fn finish_registration(
        &self,
        user_id: &str,
        user_name: &str,
        reg: &RegisterPublicKeyCredential,
    ) -> Result<StoredPasskey> {
        // Retrieve registration state
        let mut challenges = self.registration_challenges.lock().await;
        let reg_state = challenges.remove(user_id)
            .context("No registration challenge found for user")?;

        // Verify registration
        let passkey = self.webauthn
            .finish_passkey_registration(reg, &reg_state)
            .context("Failed to verify passkey registration")?;

        Ok(StoredPasskey {
            credential: passkey,
            user_name: user_name.to_string(),
            created_at: chrono::Utc::now().timestamp(),
            last_used: None,
        })
    }

    /// Start passkey authentication for a user
    ///
    /// Returns a challenge that should be sent to the client for authentication.
    pub async fn start_authentication(
        &self,
        user_id: &str,
        credentials: Vec<Passkey>,
    ) -> Result<RequestChallengeResponse> {
        if credentials.is_empty() {
            anyhow::bail!("No passkeys registered for user");
        }

        let (rcr, auth_state) = self.webauthn
            .start_passkey_authentication(&credentials)
            .context("Failed to start passkey authentication")?;

        // Store authentication state
        let mut challenges = self.auth_challenges.lock().await;
        challenges.insert(user_id.to_string(), auth_state);

        Ok(rcr)
    }

    /// Complete passkey authentication
    ///
    /// Verifies the client's authentication response and returns updated credential.
    pub async fn finish_authentication(
        &self,
        user_id: &str,
        auth: &PublicKeyCredential,
        credentials: &[Passkey],
    ) -> Result<AuthenticationResult> {
        // Retrieve authentication state
        let mut challenges = self.auth_challenges.lock().await;
        let auth_state = challenges.remove(user_id)
            .context("No authentication challenge found for user")?;

        // Verify authentication
        let auth_result = self.webauthn
            .finish_passkey_authentication(auth, &auth_state)
            .context("Failed to verify passkey authentication")?;

        // Find which credential was used
        let used_cred_id = auth_result.cred_id();
        let _credential_index = credentials
            .iter()
            .position(|pk| pk.cred_id() == used_cred_id);

        Ok(auth_result)
    }

    /// Clean up expired challenges (should be called periodically)
    pub async fn cleanup_expired_challenges(&self) {
        // Registration challenges expire after 5 minutes
        let mut reg_challenges = self.registration_challenges.lock().await;
        reg_challenges.clear(); // Simple approach: clear all periodically

        let mut auth_challenges = self.auth_challenges.lock().await;
        auth_challenges.clear();
    }
}

/// WebAuthn registration request message
#[derive(Debug, Serialize, Deserialize)]
pub struct WebAuthnRegistrationRequest {
    pub user_id: String,
    pub user_name: String,
}

/// WebAuthn authentication request message
#[derive(Debug, Serialize, Deserialize)]
pub struct WebAuthnAuthRequest {
    pub user_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webauthn_creation() {
        // Test with localhost (common development scenario)
        let result = WebAuthnAuth::new("localhost", "https://localhost:8443");
        assert!(result.is_ok(), "Failed to create WebAuthn for localhost");
    }

    #[test]
    fn test_webauthn_invalid_origin() {
        // Test with invalid origin
        let result = WebAuthnAuth::new("localhost", "not-a-url");
        assert!(result.is_err(), "Should fail with invalid origin");
    }

    #[tokio::test]
    async fn test_registration_flow() {
        let webauthn = WebAuthnAuth::new("localhost", "https://localhost:8443")
            .expect("Failed to create WebAuthn");

        let user_id = Uuid::new_v4().to_string();
        let user_name = "test_user";

        // Start registration
        let challenge = webauthn
            .start_registration(&user_id, user_name, vec![])
            .await
            .expect("Failed to start registration");

        // Verify challenge structure
        // Challenge should be present (can't access private field directly)
        assert_eq!(challenge.public_key.rp.name, "RustyClaw");
        assert_eq!(challenge.public_key.user.name, user_name);
    }

    #[tokio::test]
    async fn test_challenge_cleanup() {
        let webauthn = WebAuthnAuth::new("localhost", "https://localhost:8443")
            .expect("Failed to create WebAuthn");

        let user_id = Uuid::new_v4().to_string();

        // Start registration to create a challenge
        let _ = webauthn
            .start_registration(&user_id, "test", vec![])
            .await
            .expect("Failed to start registration");

        // Verify challenge exists
        {
            let challenges = webauthn.registration_challenges.lock().await;
            assert!(challenges.contains_key(&user_id));
        }

        // Cleanup
        webauthn.cleanup_expired_challenges().await;

        // Verify challenge removed
        {
            let challenges = webauthn.registration_challenges.lock().await;
            assert!(!challenges.contains_key(&user_id));
        }
    }
}
