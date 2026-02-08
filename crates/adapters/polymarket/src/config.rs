//! Configuration for the Polymarket execution client.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use nautilus_model::identifiers::{AccountId, TraderId, Venue};
use serde::{Deserialize, Serialize};

/// Shared map from instrument symbol (e.g. "BTC-15min-DOWN") to Polymarket CTF token ID.
pub type TokenMap = Arc<RwLock<HashMap<String, String>>>;

/// Shared map from client_order_id → CLOB acceptance result (true=accepted, false=rejected).
/// Written by the async submit_order task, read by the fill confirmation logic.
pub type OrderResultMap = Arc<RwLock<HashMap<String, bool>>>;

/// Configuration for `PolymarketExecutionClient` instances.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolymarketExecutionClientConfig {
    /// The trader ID for this client.
    pub trader_id: TraderId,
    /// The account ID for this client.
    pub account_id: AccountId,
    /// The venue identifier.
    pub venue: Venue,
    /// Ethereum private key (hex, without 0x prefix) for EIP-712 signing.
    pub private_key: String,
    /// Polymarket CLOB API key.
    pub api_key: String,
    /// Polymarket CLOB API secret.
    pub api_secret: String,
    /// Polymarket CLOB API passphrase.
    pub api_passphrase: String,
    /// CLOB REST API base URL.
    pub rest_url: String,
    /// WebSocket URL for real-time updates.
    pub ws_url: String,
    /// Chain ID (137 for Polygon mainnet, 80002 for Amoy testnet).
    pub chain_id: u64,
    /// CTF Exchange contract address.
    pub ctf_exchange_address: String,
    /// Maximum position size in USD (safety limit).
    pub max_position_usd: f64,
    /// Signature type for CLOB API: 0 = EOA, 1 = POLY_PROXY, 2 = POLY_GNOSIS_SAFE.
    pub signature_type: u8,
    /// Optional funder address (e.g. Gnosis Safe proxy) to use as `maker` in orders.
    /// When set, orders use this as `maker` while the derived EOA remains the `signer`.
    /// Required for signature_type=2 (POLY_GNOSIS_SAFE).
    pub funder_address: Option<String>,
    /// Shared token map: instrument symbol → Polymarket CTF token ID.
    /// Populated externally by a background resolver.
    #[serde(skip)]
    pub token_map: Option<TokenMap>,
    /// Shared order result map: client_order_id → accepted (true/false).
    /// Written by the async submit task, read by fill confirmation.
    #[serde(skip)]
    pub order_results: Option<OrderResultMap>,
}

impl Default for PolymarketExecutionClientConfig {
    fn default() -> Self {
        Self {
            trader_id: TraderId::from("POLYMARKET-001"),
            account_id: AccountId::from("POLYMARKET-001"),
            venue: Venue::from("POLYMARKET"),
            private_key: String::new(),
            api_key: String::new(),
            api_secret: String::new(),
            api_passphrase: String::new(),
            rest_url: "https://clob.polymarket.com".to_string(),
            ws_url: "wss://ws-subscriptions-clob.polymarket.com/ws/market".to_string(),
            chain_id: 137,
            ctf_exchange_address: "0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E".to_string(),
            max_position_usd: 500.0,
            signature_type: 0,
            funder_address: None,
            token_map: None,
            order_results: None,
        }
    }
}

impl PolymarketExecutionClientConfig {
    /// Creates a new config with required fields.
    #[must_use]
    pub fn new(
        trader_id: TraderId,
        account_id: AccountId,
        venue: Venue,
        private_key: String,
        api_key: String,
        api_secret: String,
        api_passphrase: String,
    ) -> Self {
        Self {
            trader_id,
            account_id,
            venue,
            private_key,
            api_key,
            api_secret,
            api_passphrase,
            ..Default::default()
        }
    }

    /// Set the REST API URL.
    #[must_use]
    pub fn with_rest_url(mut self, url: String) -> Self {
        self.rest_url = url;
        self
    }

    /// Set the WebSocket URL.
    #[must_use]
    pub fn with_ws_url(mut self, url: String) -> Self {
        self.ws_url = url;
        self
    }

    /// Set the chain ID.
    #[must_use]
    pub fn with_chain_id(mut self, chain_id: u64) -> Self {
        self.chain_id = chain_id;
        self
    }

    /// Set the max position size.
    #[must_use]
    pub fn with_max_position_usd(mut self, max: f64) -> Self {
        self.max_position_usd = max;
        self
    }

    /// Set the signature type (0=EOA, 1=POLY_PROXY, 2=POLY_GNOSIS_SAFE).
    #[must_use]
    pub fn with_signature_type(mut self, sig_type: u8) -> Self {
        self.signature_type = sig_type;
        self
    }

    /// Set the funder address (e.g. Gnosis Safe proxy) for order `maker` field.
    #[must_use]
    pub fn with_funder_address(mut self, addr: String) -> Self {
        self.funder_address = Some(addr);
        self
    }

    /// Set the shared token map for resolving instrument symbols to Polymarket CTF token IDs.
    #[must_use]
    pub fn with_token_map(mut self, token_map: TokenMap) -> Self {
        self.token_map = Some(token_map);
        self
    }

    /// Set the shared order result map for communicating CLOB acceptance back to the strategy.
    #[must_use]
    pub fn with_order_results(mut self, order_results: OrderResultMap) -> Self {
        self.order_results = Some(order_results);
        self
    }
}
