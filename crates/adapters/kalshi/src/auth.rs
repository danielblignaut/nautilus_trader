//! RSA-PSS authentication for the Kalshi API.
//!
//! Kalshi uses RSA-PSS (SHA-256) signatures for API authentication.
//! The signing message is: `{timestamp_ms}{METHOD}{path}`.

use anyhow::{anyhow, Result};
use base64::Engine;
use rsa::pkcs1v15::SigningKey;
use rsa::signature::SignatureEncoding;
use rsa::signature::Signer;
use rsa::RsaPrivateKey;

/// Kalshi API authentication using RSA-PSS signatures.
#[derive(Clone)]
pub struct KalshiAuth {
    // Note: SigningKey doesn't impl Debug, so we impl Debug manually below.
    api_key: String,
    signing_key: SigningKey<sha2::Sha256>,
}

impl std::fmt::Debug for KalshiAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KalshiAuth")
            .field("api_key", &format!("{}...", &self.api_key[..8.min(self.api_key.len())]))
            .finish()
    }
}

impl KalshiAuth {
    /// Create from API key string and PEM-encoded RSA private key.
    ///
    /// # Errors
    ///
    /// Returns an error if the PEM key cannot be parsed.
    pub fn new(api_key: String, private_key_pem: &str) -> Result<Self> {
        use rsa::pkcs8::DecodePrivateKey;
        let rsa_key = RsaPrivateKey::from_pkcs8_pem(private_key_pem)
            .map_err(|e| anyhow!("Failed to parse Kalshi RSA private key: {e}"))?;
        let signing_key = SigningKey::<sha2::Sha256>::new(rsa_key);
        Ok(Self {
            api_key,
            signing_key,
        })
    }

    /// Load from environment variables.
    ///
    /// Reads `KALSHI_API_KEY` and `KALSHI_PRIVATE_KEY` (inline PEM with `\n`
    /// for line breaks). Falls back to `KALSHI_PRIVATE_KEY_PATH` (file path).
    ///
    /// # Errors
    ///
    /// Returns an error if environment variables are missing or key is invalid.
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("KALSHI_API_KEY")
            .map_err(|_| anyhow!("Missing KALSHI_API_KEY environment variable"))?;

        let pem = if let Ok(raw) = std::env::var("KALSHI_PRIVATE_KEY") {
            raw.replace("\\n", "\n")
        } else if let Ok(path) = std::env::var("KALSHI_PRIVATE_KEY_PATH") {
            std::fs::read_to_string(&path)
                .map_err(|e| anyhow!("Failed to read Kalshi private key from {path}: {e}"))?
        } else {
            return Err(anyhow!(
                "Missing KALSHI_PRIVATE_KEY or KALSHI_PRIVATE_KEY_PATH environment variable"
            ));
        };

        Self::new(api_key, &pem)
    }

    /// Get the API key.
    #[must_use]
    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    /// Sign a request and return auth headers.
    ///
    /// Kalshi signing: `timestamp_ms + METHOD + path` signed with RSA PKCS#1v15 (SHA-256).
    pub fn sign_request(&self, method: &str, path: &str) -> AuthHeaders {
        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let message = format!("{}{}{}", timestamp_ms, method.to_uppercase(), path);
        let signature = self.signing_key.sign(message.as_bytes());
        let sig_b64 = base64::engine::general_purpose::STANDARD.encode(signature.to_bytes());

        AuthHeaders {
            api_key: self.api_key.clone(),
            signature: sig_b64,
            timestamp: timestamp_ms.to_string(),
        }
    }
}

/// Authentication headers for Kalshi API requests.
pub struct AuthHeaders {
    pub api_key: String,
    pub signature: String,
    pub timestamp: String,
}

impl AuthHeaders {
    /// Header name constants.
    pub const KALSHI_ACCESS_KEY: &'static str = "KALSHI-ACCESS-KEY";
    pub const KALSHI_ACCESS_SIGNATURE: &'static str = "KALSHI-ACCESS-SIGNATURE";
    pub const KALSHI_ACCESS_TIMESTAMP: &'static str = "KALSHI-ACCESS-TIMESTAMP";
}

/// Apply auth headers to an async reqwest request builder.
pub fn apply_auth(
    builder: reqwest::RequestBuilder,
    headers: &AuthHeaders,
) -> reqwest::RequestBuilder {
    builder
        .header(AuthHeaders::KALSHI_ACCESS_KEY, &headers.api_key)
        .header(AuthHeaders::KALSHI_ACCESS_SIGNATURE, &headers.signature)
        .header(AuthHeaders::KALSHI_ACCESS_TIMESTAMP, &headers.timestamp)
}
