//! Kalshi CLOB API request/response types.

use serde::{Deserialize, Serialize};

/// Order submission request for the Kalshi REST API.
#[derive(Debug, Clone, Serialize)]
pub struct CreateOrderRequest {
    pub ticker: String,
    pub side: String,    // "yes" or "no"
    pub action: String,  // "buy" or "sell"
    pub count: u32,      // number of contracts
    #[serde(rename = "type")]
    pub order_type: String, // "limit" or "market"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub yes_price: Option<u32>, // cents (1-99)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_price: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiration_ts: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_in_force: Option<String>, // "ioc", "gtc"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subaccount: Option<i32>,
}

/// Order submission response from the Kalshi REST API.
#[derive(Debug, Clone, Deserialize)]
pub struct OrderResponse {
    pub order: OrderInfo,
}

/// Order info from the Kalshi REST API.
#[derive(Debug, Clone, Deserialize)]
pub struct OrderInfo {
    pub order_id: String,
    pub ticker: String,
    pub status: String,
    pub side: String,
    pub action: String,
    pub yes_price: Option<u32>,
    pub no_price: Option<u32>,
    pub remaining_count: Option<u32>,
}

/// Cancel order response from the Kalshi REST API.
#[derive(Debug, Clone, Deserialize)]
pub struct CancelOrderResponse {
    pub order: OrderInfo,
}

/// Balance response from the Kalshi REST API.
#[derive(Debug, Deserialize)]
pub struct BalanceResponse {
    pub balance: i64, // cents
}
