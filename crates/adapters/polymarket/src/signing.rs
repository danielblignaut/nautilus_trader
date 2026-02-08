//! EIP-712 order signing for Polymarket's CTF Exchange.
//!
//! Implements the structured data signing required by Polymarket's
//! on-chain settlement contract. Orders must be signed with the maker's
//! Ethereum private key using EIP-712 typed data.
//!
//! Also provides HMAC-SHA256 computation for CLOB API authentication.

use alloy_primitives::{Address, B256, U256};
use alloy_sol_types::sol;
use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use hmac::{Hmac, Mac};
use k256::ecdsa::SigningKey;
use sha2::Sha256;
use sha3::{Digest, Keccak256};

/// EIP-712 domain separator for Polymarket CTF Exchange.
#[derive(Debug, Clone)]
pub struct Eip712Domain {
    pub name: String,
    pub version: String,
    pub chain_id: u64,
    pub verifying_contract: String,
}

impl Default for Eip712Domain {
    fn default() -> Self {
        Self {
            name: "Polymarket CTF Exchange".to_string(),
            version: "1".to_string(),
            chain_id: 137, // Polygon mainnet
            verifying_contract: "0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E".to_string(),
        }
    }
}

impl Eip712Domain {
    /// Create a domain for Amoy testnet (chain_id = 80002).
    #[must_use]
    pub fn amoy() -> Self {
        Self {
            chain_id: 80002,
            ..Default::default()
        }
    }

    /// Compute the EIP-712 domain separator hash.
    pub fn separator(&self) -> Result<B256> {
        // EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)
        let type_hash = keccak256(
            b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
        );

        let name_hash = keccak256(self.name.as_bytes());
        let version_hash = keccak256(self.version.as_bytes());
        let chain_id = U256::from(self.chain_id);
        let verifying_contract: Address = self
            .verifying_contract
            .parse()
            .map_err(|e| anyhow!("Invalid verifying contract address: {e}"))?;

        // Encode: typeHash || nameHash || versionHash || chainId || verifyingContract
        let mut encoded = Vec::with_capacity(160);
        encoded.extend_from_slice(type_hash.as_slice());
        encoded.extend_from_slice(name_hash.as_slice());
        encoded.extend_from_slice(version_hash.as_slice());
        encoded.extend_from_slice(&chain_id.to_be_bytes::<32>());
        encoded.extend_from_slice(&[0u8; 12]); // left-pad address to 32 bytes
        encoded.extend_from_slice(verifying_contract.as_slice());

        Ok(keccak256(&encoded))
    }
}

/// EIP-712 order struct for signing.
///
/// Matches the Polymarket CTF Exchange Order struct:
/// ```solidity
/// Order(uint256 salt,address maker,address signer,address taker,
///       uint256 tokenId,uint256 makerAmount,uint256 takerAmount,
///       uint256 expiration,uint256 nonce,uint256 feeRateBps,
///       uint8 side,uint8 signatureType)
/// ```
#[derive(Debug, Clone)]
pub struct OrderData {
    pub salt: String,
    pub maker: String,
    pub signer: String,
    pub taker: String,
    pub token_id: String,
    pub maker_amount: String,
    pub taker_amount: String,
    pub expiration: String,
    pub nonce: String,
    pub fee_rate_bps: String,
    pub side: u8,           // 0 = BUY, 1 = SELL
    pub signature_type: u8, // 0 = EOA, 1 = Polymarket proxy, 2 = proxy + split
}

impl OrderData {
    /// Zero address for public orders (taker can be anyone).
    pub const ZERO_ADDRESS: &'static str = "0x0000000000000000000000000000000000000000";
}

// Define the Order type using alloy's sol! macro for correct ABI encoding
sol! {
    #[derive(Debug)]
    struct Order {
        uint256 salt;
        address maker;
        address signer;
        address taker;
        uint256 tokenId;
        uint256 makerAmount;
        uint256 takerAmount;
        uint256 expiration;
        uint256 nonce;
        uint256 feeRateBps;
        uint8 side;
        uint8 signatureType;
    }
}

/// Order type hash for EIP-712.
const ORDER_TYPE_HASH: &[u8] = b"Order(uint256 salt,address maker,address signer,address taker,uint256 tokenId,uint256 makerAmount,uint256 takerAmount,uint256 expiration,uint256 nonce,uint256 feeRateBps,uint8 side,uint8 signatureType)";

/// Sign an order using EIP-712 typed data.
///
/// Returns the hex-encoded signature (65 bytes: r || s || v).
///
/// # Errors
///
/// Returns an error if the private key is invalid or signing fails.
pub fn sign_order(private_key: &str, domain: &Eip712Domain, order: &OrderData) -> Result<String> {
    // Parse private key (strip 0x prefix if present)
    let key_hex = private_key.strip_prefix("0x").unwrap_or(private_key);
    let key_bytes = hex::decode(key_hex).map_err(|e| anyhow!("Invalid private key hex: {e}"))?;
    let signing_key =
        SigningKey::from_bytes((&key_bytes[..]).into()).map_err(|e| anyhow!("Invalid private key: {e}"))?;

    // Compute domain separator
    let domain_separator = domain.separator()?;

    // Compute struct hash
    let struct_hash = compute_order_struct_hash(order)?;

    // Compute EIP-712 digest: keccak256("\x19\x01" || domainSeparator || structHash)
    let mut digest_input = Vec::with_capacity(66);
    digest_input.extend_from_slice(&[0x19, 0x01]);
    digest_input.extend_from_slice(domain_separator.as_slice());
    digest_input.extend_from_slice(struct_hash.as_slice());
    let digest = keccak256(&digest_input);

    log::debug!(
        "EIP-712 sign_order: domainSep=0x{} structHash=0x{} digest=0x{}",
        hex::encode(domain_separator.as_slice()),
        hex::encode(struct_hash.as_slice()),
        hex::encode(digest.as_slice()),
    );

    // Sign the digest with ECDSA
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(digest.as_slice())
        .map_err(|e| anyhow!("Signing failed: {e}"))?;

    // Encode signature as r || s || v (65 bytes)
    let r = signature.r().to_bytes();
    let s = signature.s().to_bytes();
    let v = recovery_id.to_byte() + 27; // Ethereum uses 27/28

    let mut sig_bytes = Vec::with_capacity(65);
    sig_bytes.extend_from_slice(&r);
    sig_bytes.extend_from_slice(&s);
    sig_bytes.push(v);

    Ok(format!("0x{}", hex::encode(&sig_bytes)))
}

/// Compute the struct hash for an Order.
fn compute_order_struct_hash(order: &OrderData) -> Result<B256> {
    let type_hash = keccak256(ORDER_TYPE_HASH);

    // Parse all fields
    let salt = parse_u256(&order.salt)?;
    let maker: Address = order
        .maker
        .parse()
        .map_err(|e| anyhow!("Invalid maker address: {e}"))?;
    let signer: Address = order
        .signer
        .parse()
        .map_err(|e| anyhow!("Invalid signer address: {e}"))?;
    let taker: Address = order
        .taker
        .parse()
        .map_err(|e| anyhow!("Invalid taker address: {e}"))?;
    let token_id = parse_u256(&order.token_id)?;
    let maker_amount = parse_u256(&order.maker_amount)?;
    let taker_amount = parse_u256(&order.taker_amount)?;
    let expiration = parse_u256(&order.expiration)?;
    let nonce = parse_u256(&order.nonce)?;
    let fee_rate_bps = parse_u256(&order.fee_rate_bps)?;

    // Encode: typeHash || salt || maker || signer || taker || tokenId || makerAmount ||
    //         takerAmount || expiration || nonce || feeRateBps || side || signatureType
    // Note: uint8 values are left-padded to 32 bytes
    let mut encoded = Vec::with_capacity(416);
    encoded.extend_from_slice(type_hash.as_slice());
    encoded.extend_from_slice(&salt.to_be_bytes::<32>());
    encoded.extend_from_slice(&[0u8; 12]); // padding for address
    encoded.extend_from_slice(maker.as_slice());
    encoded.extend_from_slice(&[0u8; 12]); // padding for address
    encoded.extend_from_slice(signer.as_slice());
    encoded.extend_from_slice(&[0u8; 12]); // padding for address
    encoded.extend_from_slice(taker.as_slice());
    encoded.extend_from_slice(&token_id.to_be_bytes::<32>());
    encoded.extend_from_slice(&maker_amount.to_be_bytes::<32>());
    encoded.extend_from_slice(&taker_amount.to_be_bytes::<32>());
    encoded.extend_from_slice(&expiration.to_be_bytes::<32>());
    encoded.extend_from_slice(&nonce.to_be_bytes::<32>());
    encoded.extend_from_slice(&fee_rate_bps.to_be_bytes::<32>());
    // uint8 side - left-padded to 32 bytes
    encoded.extend_from_slice(&[0u8; 31]);
    encoded.push(order.side);
    // uint8 signatureType - left-padded to 32 bytes
    encoded.extend_from_slice(&[0u8; 31]);
    encoded.push(order.signature_type);

    Ok(keccak256(&encoded))
}

/// Parse a string to U256, handling decimal and hex formats.
fn parse_u256(s: &str) -> Result<U256> {
    if s.starts_with("0x") || s.starts_with("0X") {
        U256::from_str_radix(&s[2..], 16).map_err(|e| anyhow!("Invalid hex U256: {e}"))
    } else {
        U256::from_str_radix(s, 10).map_err(|e| anyhow!("Invalid decimal U256: {e}"))
    }
}

/// Compute keccak256 hash.
fn keccak256(data: &[u8]) -> B256 {
    let mut hasher = Keccak256::new();
    hasher.update(data);
    B256::from_slice(&hasher.finalize())
}

/// Generate a random salt for order signing.
///
/// Uses the current timestamp in nanoseconds as the salt.
#[must_use]
pub fn generate_salt() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    // Use nanosecond timestamp truncated to u32 range (matching py-clob-client's small salt range)
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Combine seconds and nanos for uniqueness, keep in u32 range
    ((secs ^ (nanos as u64)) & 0xFFFF_FFFF) as u64
}

/// Compute the HMAC-SHA256 signature for CLOB API authentication.
///
/// The signature is computed as:
/// ```text
/// signature = HMAC-SHA256(base64_decode(secret), timestamp + method + path + body)
/// ```
///
/// Returns the base64-encoded signature.
///
/// # Errors
///
/// Returns an error if the secret is invalid base64.
pub fn compute_api_signature(
    api_secret: &str,
    timestamp: &str,
    method: &str,
    path: &str,
    body: &str,
) -> Result<String> {
    // Decode the base64-encoded secret.
    // Polymarket may return secrets in base64url encoding (with - and _ instead
    // of + and /), so normalize to standard base64 before decoding.
    let normalized = api_secret.replace('-', "+").replace('_', "/");
    let secret_bytes = BASE64
        .decode(&normalized)
        .map_err(|e| anyhow!("Invalid API secret (not valid base64): {e}"))?;

    // Create HMAC-SHA256 instance
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(&secret_bytes)
        .map_err(|e| anyhow!("Invalid HMAC key length: {e}"))?;

    // Build the message: timestamp + method + path + body
    let message = format!("{timestamp}{method}{path}{body}");
    mac.update(message.as_bytes());

    // Get the signature and base64url-encode it (Polymarket expects URL-safe base64)
    let signature = mac.finalize().into_bytes();
    let encoded = BASE64.encode(signature);
    Ok(encoded.replace('+', "-").replace('/', "_"))
}

/// Build the L2 authentication headers for CLOB API requests.
///
/// Returns all 6 headers required by Polymarket: POLY_ADDRESS, POLY_API_KEY,
/// POLY_SIGNATURE, POLY_TIMESTAMP, POLY_NONCE, POLY_PASSPHRASE.
pub fn build_l2_headers(
    address: &str,
    api_key: &str,
    api_secret: &str,
    api_passphrase: &str,
    method: &str,
    path: &str,
    body: &str,
) -> Result<L2Headers> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string();

    let signature = compute_api_signature(api_secret, &timestamp, method, path, body)?;

    Ok(L2Headers {
        address: address.to_string(),
        api_key: api_key.to_string(),
        signature,
        timestamp,
        nonce: "0".to_string(),
        passphrase: api_passphrase.to_string(),
    })
}

/// L2 authentication headers for CLOB API.
#[derive(Debug, Clone)]
pub struct L2Headers {
    pub address: String,
    pub api_key: String,
    pub signature: String,
    pub timestamp: String,
    pub nonce: String,
    pub passphrase: String,
}

impl L2Headers {
    /// Header name constants.
    pub const POLY_ADDRESS: &'static str = "POLY_ADDRESS";
    pub const POLY_SIGNATURE: &'static str = "POLY_SIGNATURE";
    pub const POLY_TIMESTAMP: &'static str = "POLY_TIMESTAMP";
    pub const POLY_NONCE: &'static str = "POLY_NONCE";
    pub const POLY_API_KEY: &'static str = "POLY_API_KEY";
    pub const POLY_PASSPHRASE: &'static str = "POLY_PASSPHRASE";
}

// ---------------------------------------------------------------------------
// L1 CLOB Auth (EIP-712) — used to derive/create API keys from private key
// ---------------------------------------------------------------------------

/// EIP-712 domain for CLOB L1 authentication.
///
/// Different from the CTF Exchange domain: no `verifyingContract`, different
/// name ("ClobAuthDomain").
#[derive(Debug, Clone)]
pub struct ClobAuthDomain {
    pub chain_id: u64,
}

impl ClobAuthDomain {
    /// Compute the EIP-712 domain separator for CLOB auth.
    ///
    /// Type: `EIP712Domain(string name,string version,uint256 chainId)`
    pub fn separator(&self) -> B256 {
        let type_hash =
            keccak256(b"EIP712Domain(string name,string version,uint256 chainId)");
        let name_hash = keccak256(b"ClobAuthDomain");
        let version_hash = keccak256(b"1");
        let chain_id = U256::from(self.chain_id);

        let mut encoded = Vec::with_capacity(128);
        encoded.extend_from_slice(type_hash.as_slice());
        encoded.extend_from_slice(name_hash.as_slice());
        encoded.extend_from_slice(version_hash.as_slice());
        encoded.extend_from_slice(&chain_id.to_be_bytes::<32>());

        keccak256(&encoded)
    }
}

/// ClobAuth struct hash for L1 auth.
///
/// Type: `ClobAuth(address address,string timestamp,uint256 nonce,string message)`
fn compute_clob_auth_struct_hash(address: &Address, timestamp: &str, nonce: u64) -> B256 {
    let type_hash = keccak256(
        b"ClobAuth(address address,string timestamp,uint256 nonce,string message)",
    );
    let timestamp_hash = keccak256(timestamp.as_bytes());
    let nonce_u256 = U256::from(nonce);
    let message_hash =
        keccak256(b"This message attests that I control the given wallet");

    let mut encoded = Vec::with_capacity(192);
    encoded.extend_from_slice(type_hash.as_slice());
    // address — left-padded to 32 bytes
    encoded.extend_from_slice(&[0u8; 12]);
    encoded.extend_from_slice(address.as_slice());
    encoded.extend_from_slice(timestamp_hash.as_slice());
    encoded.extend_from_slice(&nonce_u256.to_be_bytes::<32>());
    encoded.extend_from_slice(message_hash.as_slice());

    keccak256(&encoded)
}

/// L1 authentication headers for the CLOB API.
#[derive(Debug, Clone)]
pub struct L1Headers {
    pub address: String,
    pub signature: String,
    pub timestamp: String,
    pub nonce: String,
}

impl L1Headers {
    pub const POLY_ADDRESS: &'static str = "POLY_ADDRESS";
    pub const POLY_SIGNATURE: &'static str = "POLY_SIGNATURE";
    pub const POLY_TIMESTAMP: &'static str = "POLY_TIMESTAMP";
    pub const POLY_NONCE: &'static str = "POLY_NONCE";
}

/// Sign an L1 CLOB auth message using EIP-712.
///
/// Returns [`L1Headers`] containing the address, hex-encoded signature,
/// timestamp and nonce ready to be sent as HTTP headers.
pub fn sign_clob_auth(private_key: &str, chain_id: u64) -> Result<L1Headers> {
    use std::time::{SystemTime, UNIX_EPOCH};

    // Parse private key
    let key_hex = private_key.strip_prefix("0x").unwrap_or(private_key);
    let key_bytes =
        hex::decode(key_hex).map_err(|e| anyhow!("Invalid private key hex: {e}"))?;
    let signing_key = SigningKey::from_bytes((&key_bytes[..]).into())
        .map_err(|e| anyhow!("Invalid private key: {e}"))?;

    // Derive address
    let address_str = derive_address(private_key)?;
    let address: Address = address_str
        .parse()
        .map_err(|e| anyhow!("Invalid derived address: {e}"))?;

    // Timestamp (seconds) and nonce
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string();
    let nonce: u64 = 0;

    // Domain separator + struct hash → EIP-712 digest
    let domain = ClobAuthDomain { chain_id };
    let domain_separator = domain.separator();
    let struct_hash = compute_clob_auth_struct_hash(&address, &timestamp, nonce);

    let mut digest_input = Vec::with_capacity(66);
    digest_input.extend_from_slice(&[0x19, 0x01]);
    digest_input.extend_from_slice(domain_separator.as_slice());
    digest_input.extend_from_slice(struct_hash.as_slice());
    let digest = keccak256(&digest_input);

    // ECDSA sign
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(digest.as_slice())
        .map_err(|e| anyhow!("Signing failed: {e}"))?;

    // r || s || v (65 bytes)
    let r = signature.r().to_bytes();
    let s = signature.s().to_bytes();
    let v = recovery_id.to_byte() + 27;

    let mut sig_bytes = Vec::with_capacity(65);
    sig_bytes.extend_from_slice(&r);
    sig_bytes.extend_from_slice(&s);
    sig_bytes.push(v);

    Ok(L1Headers {
        address: address_str,
        signature: format!("0x{}", hex::encode(&sig_bytes)),
        timestamp,
        nonce: nonce.to_string(),
    })
}

/// Derive the Ethereum address from a private key.
pub fn derive_address(private_key: &str) -> Result<String> {
    let key_hex = private_key.strip_prefix("0x").unwrap_or(private_key);
    let key_bytes = hex::decode(key_hex).map_err(|e| anyhow!("Invalid private key hex: {e}"))?;
    let signing_key =
        SigningKey::from_bytes((&key_bytes[..]).into()).map_err(|e| anyhow!("Invalid private key: {e}"))?;

    let verifying_key = signing_key.verifying_key();
    let public_key_bytes = verifying_key.to_encoded_point(false);
    let public_key_uncompressed = &public_key_bytes.as_bytes()[1..]; // Skip the 0x04 prefix

    let mut hasher = Keccak256::new();
    hasher.update(public_key_uncompressed);
    let hash = hasher.finalize();

    // Take last 20 bytes as address, apply EIP-55 checksum casing
    let address = &hash[12..];
    Ok(to_checksum_address(address))
}

/// Convert raw address bytes to EIP-55 checksum-cased string.
fn to_checksum_address(address_bytes: &[u8]) -> String {
    let hex_addr = hex::encode(address_bytes);
    let mut hasher = Keccak256::new();
    hasher.update(hex_addr.as_bytes());
    let hash = hasher.finalize();

    let mut checksummed = String::with_capacity(42);
    checksummed.push_str("0x");
    for (i, c) in hex_addr.chars().enumerate() {
        let hash_nibble = if i % 2 == 0 {
            (hash[i / 2] >> 4) & 0xf
        } else {
            hash[i / 2] & 0xf
        };
        if hash_nibble >= 8 {
            checksummed.push(c.to_ascii_uppercase());
        } else {
            checksummed.push(c);
        }
    }
    checksummed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_salt() {
        let salt = generate_salt();
        assert!(salt > 0);
        assert!(salt <= u32::MAX as u64);
    }

    #[test]
    fn test_default_domain() {
        let domain = Eip712Domain::default();
        assert_eq!(domain.name, "Polymarket CTF Exchange");
        assert_eq!(domain.chain_id, 137);
    }

    #[test]
    fn test_amoy_domain() {
        let domain = Eip712Domain::amoy();
        assert_eq!(domain.name, "Polymarket CTF Exchange");
        assert_eq!(domain.chain_id, 80002);
    }

    #[test]
    fn test_domain_separator() {
        let domain = Eip712Domain::default();
        let separator = domain.separator().unwrap();
        // Verify it's a 32-byte hash
        assert_eq!(separator.len(), 32);
    }

    #[test]
    fn test_derive_address() {
        // Test with a known private key (do not use in production!)
        let private_key = "0x0000000000000000000000000000000000000000000000000000000000000001";
        let address = derive_address(private_key).unwrap();
        // Known address for this private key
        assert_eq!(
            address.to_lowercase(),
            "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf"
        );
    }

    #[test]
    fn test_compute_api_signature() {
        // Test with known values
        let secret = BASE64.encode(b"test_secret");
        let result = compute_api_signature(&secret, "1234567890", "POST", "/order", "{}");
        assert!(result.is_ok());
        let sig = result.unwrap();
        // Output is base64url encoded — normalize back to standard for decode check
        let normalized = sig.replace('-', "+").replace('_', "/");
        assert!(BASE64.decode(&normalized).is_ok());
    }

    #[test]
    fn test_sign_order() {
        // Test signing with a known private key
        let private_key = "0000000000000000000000000000000000000000000000000000000000000001";
        let domain = Eip712Domain::default();
        let order = OrderData {
            salt: "12345".to_string(),
            maker: "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf".to_string(),
            signer: "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf".to_string(),
            taker: OrderData::ZERO_ADDRESS.to_string(),
            token_id: "123456789".to_string(),
            maker_amount: "1000000".to_string(), // 1 USDC (6 decimals)
            taker_amount: "1000000".to_string(),
            expiration: "0".to_string(),
            nonce: "0".to_string(),
            fee_rate_bps: "0".to_string(),
            side: 0, // BUY
            signature_type: 0,
        };

        let result = sign_order(private_key, &domain, &order);
        assert!(result.is_ok());
        let sig = result.unwrap();
        // Signature should be 65 bytes hex encoded with 0x prefix
        assert!(sig.starts_with("0x"));
        assert_eq!(sig.len(), 132); // "0x" + 130 hex chars
    }

    #[test]
    fn test_sign_order_crosscheck_python() {
        // Cross-validate with Python: same inputs should produce same signature
        let private_key = "652dda0d99c3ee389ec94b391e4693ad318fa02d7f038e543914d6ee59486857";
        let domain = Eip712Domain::default();
        let order = OrderData {
            salt: "12345".to_string(),
            maker: "0xF68A8dFe2b03DCD7116278e9c0591AE57Dc0b36D".to_string(),
            signer: "0xF68A8dFe2b03DCD7116278e9c0591AE57Dc0b36D".to_string(),
            taker: OrderData::ZERO_ADDRESS.to_string(),
            token_id: "123456789".to_string(),
            maker_amount: "1000000".to_string(),
            taker_amount: "2000000".to_string(),
            expiration: "0".to_string(),
            nonce: "0".to_string(),
            fee_rate_bps: "1000".to_string(),
            side: 0,
            signature_type: 0,
        };

        // Domain separator should match Python (name="Polymarket CTF Exchange"):
        // 0x1a573e3617c78403b5b4b892827992f027b03d4eaf570048b8ee8cdd84d151be
        let domain_sep = domain.separator().unwrap();
        let domain_sep_hex = hex::encode(domain_sep.as_slice());
        eprintln!("Rust domainSeparator: 0x{}", domain_sep_hex);
        assert_eq!(domain_sep_hex, "1a573e3617c78403b5b4b892827992f027b03d4eaf570048b8ee8cdd84d151be");

        // Struct hash should match Python (struct hash is domain-independent):
        // 0x4c8e8e2f51e28026c4deef1e9327ad3ac6263ec61df26b2a118d320802818c06
        let struct_hash = compute_order_struct_hash(&order).unwrap();
        let struct_hash_hex = hex::encode(struct_hash.as_slice());
        eprintln!("Rust structHash: 0x{}", struct_hash_hex);
        assert_eq!(struct_hash_hex, "4c8e8e2f51e28026c4deef1e9327ad3ac6263ec61df26b2a118d320802818c06");

        // Full signature should match Python with correct domain name:
        // 0x0ce6b78aabf2db450c7a7277310c90722b842aace5dd33029cff0bb685f90f3b13085e96537c5d224713c49b1d7c4473062bbbc5661de9b507d4919a05ccce011b
        let sig = sign_order(private_key, &domain, &order).unwrap();
        eprintln!("Rust signature: {}", sig);
        assert_eq!(sig, "0x0ce6b78aabf2db450c7a7277310c90722b842aace5dd33029cff0bb685f90f3b13085e96537c5d224713c49b1d7c4473062bbbc5661de9b507d4919a05ccce011b");
    }

    #[test]
    fn test_build_l2_headers() {
        let secret = BASE64.encode(b"test_secret");
        let headers = build_l2_headers(
            "0xabc123",
            "api_key",
            &secret,
            "passphrase",
            "POST",
            "/order",
            "{}",
        );
        assert!(headers.is_ok());
        let h = headers.unwrap();
        assert_eq!(h.address, "0xabc123");
        assert_eq!(h.api_key, "api_key");
        assert_eq!(h.passphrase, "passphrase");
        assert!(!h.timestamp.is_empty());
        assert_eq!(h.nonce, "0");
        assert!(!h.signature.is_empty());
    }

    #[test]
    fn test_clob_auth_domain_separator() {
        let domain = ClobAuthDomain { chain_id: 137 };
        let sep = domain.separator();
        assert_eq!(sep.len(), 32);

        // Different chain_id should produce different separator
        let domain2 = ClobAuthDomain { chain_id: 80002 };
        let sep2 = domain2.separator();
        assert_ne!(sep, sep2);
    }

    #[test]
    fn test_sign_clob_auth() {
        let private_key = "0x0000000000000000000000000000000000000000000000000000000000000001";
        let result = sign_clob_auth(private_key, 137);
        assert!(result.is_ok());
        let headers = result.unwrap();

        // Signature: 0x + 130 hex chars = 132
        assert!(headers.signature.starts_with("0x"));
        assert_eq!(headers.signature.len(), 132);

        // Address should match derive_address
        let expected_address = derive_address(private_key).unwrap();
        assert_eq!(headers.address.to_lowercase(), expected_address.to_lowercase());

        // Nonce should be "0"
        assert_eq!(headers.nonce, "0");

        // Timestamp should be a valid number
        assert!(headers.timestamp.parse::<u64>().is_ok());
    }
}
