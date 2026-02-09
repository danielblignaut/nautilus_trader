//! Configuration for the Kalshi execution client.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use nautilus_model::identifiers::{AccountId, TraderId, Venue};
use serde::{Deserialize, Serialize};

/// Shared map from instrument symbol (e.g. "BTC-15min-Up") to Kalshi market ticker.
/// Populated externally by a background ticker resolver.
pub type TickerMap = Arc<RwLock<HashMap<String, String>>>;

/// Shared map from client_order_id → CLOB acceptance result (true=accepted, false=rejected).
/// Written by the async submit_order task, read by the fill confirmation logic.
pub type OrderResultMap = Arc<RwLock<HashMap<String, bool>>>;

/// Configuration for `KalshiExecutionClient` instances.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KalshiExecutionClientConfig {
    /// The trader ID for this client.
    pub trader_id: TraderId,
    /// The account ID for this client.
    pub account_id: AccountId,
    /// The venue identifier.
    pub venue: Venue,
    /// Kalshi API key.
    pub api_key: String,
    /// RSA private key (PEM-encoded).
    pub private_key_pem: String,
    /// REST API base URL.
    pub rest_url: String,
    /// Subaccount number (0=primary, 1-32=subaccounts).
    pub subaccount: Option<i32>,
    /// Asset name (e.g. "BTC") for ticker resolution.
    pub asset: String,
    /// Maximum position size in USD (safety limit).
    pub max_position_usd: f64,
    /// Shared ticker map: instrument symbol → Kalshi market ticker.
    #[serde(skip)]
    pub ticker_map: Option<TickerMap>,
    /// Shared order result map: client_order_id → accepted (true/false).
    #[serde(skip)]
    pub order_results: Option<OrderResultMap>,
}

impl Default for KalshiExecutionClientConfig {
    fn default() -> Self {
        Self {
            trader_id: TraderId::from("KALSHI-001"),
            account_id: AccountId::from("KALSHI-001"),
            venue: Venue::from("KALSHI"),
            api_key: String::new(),
            private_key_pem: String::new(),
            rest_url: "https://api.elections.kalshi.com/trade-api/v2".to_string(),
            subaccount: None,
            asset: "BTC".to_string(),
            max_position_usd: 500.0,
            ticker_map: None,
            order_results: None,
        }
    }
}

impl KalshiExecutionClientConfig {
    /// Creates a new config with required fields.
    #[must_use]
    pub fn new(
        trader_id: TraderId,
        account_id: AccountId,
        venue: Venue,
        api_key: String,
        private_key_pem: String,
    ) -> Self {
        Self {
            trader_id,
            account_id,
            venue,
            api_key,
            private_key_pem,
            ..Default::default()
        }
    }

    /// Set the REST API URL.
    #[must_use]
    pub fn with_rest_url(mut self, url: String) -> Self {
        self.rest_url = url;
        self
    }

    /// Set the subaccount number.
    #[must_use]
    pub fn with_subaccount(mut self, subaccount: i32) -> Self {
        self.subaccount = Some(subaccount);
        self
    }

    /// Set the asset name.
    #[must_use]
    pub fn with_asset(mut self, asset: String) -> Self {
        self.asset = asset;
        self
    }

    /// Set the max position size.
    #[must_use]
    pub fn with_max_position_usd(mut self, max: f64) -> Self {
        self.max_position_usd = max;
        self
    }

    /// Set the shared ticker map for resolving instrument symbols to Kalshi market tickers.
    #[must_use]
    pub fn with_ticker_map(mut self, ticker_map: TickerMap) -> Self {
        self.ticker_map = Some(ticker_map);
        self
    }

    /// Set the shared order result map for communicating acceptance back to the strategy.
    #[must_use]
    pub fn with_order_results(mut self, order_results: OrderResultMap) -> Self {
        self.order_results = Some(order_results);
        self
    }
}
