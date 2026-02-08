//! Polymarket CLOB API request/response types.

use serde::{Deserialize, Serialize};

/// Order side for Polymarket CLOB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum PolymarketSide {
    Buy,
    Sell,
}

/// Order type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum PolymarketOrderType {
    Gtc,  // Good-til-canceled
    Fok,  // Fill-or-kill
    Ioc,  // Immediate-or-cancel
}

/// Order status from the CLOB API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PolymarketOrderStatus {
    Live,
    Matched,
    Cancelled,
    Expired,
}

/// Signed order for CLOB submission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedOrder {
    /// EIP-712 signature.
    pub signature: String,
    /// Order salt (random nonce) — must be JSON integer for CLOB API.
    pub salt: u64,
    /// Maker address.
    pub maker: String,
    /// Signer address.
    pub signer: String,
    /// Taker address (zero address for public orders).
    pub taker: String,
    /// Token ID (condition token).
    #[serde(rename = "tokenId")]
    pub token_id: String,
    /// Maker amount (in USDC base units, 6 decimals).
    #[serde(rename = "makerAmount")]
    pub maker_amount: String,
    /// Taker amount (in conditional token base units).
    #[serde(rename = "takerAmount")]
    pub taker_amount: String,
    /// Side: BUY or SELL.
    pub side: PolymarketSide,
    /// Expiration timestamp (0 = no expiration).
    pub expiration: String,
    /// Nonce.
    pub nonce: String,
    /// Fee rate bps.
    #[serde(rename = "feeRateBps")]
    pub fee_rate_bps: String,
    /// Signature type (0 = EOA, 1 = Polymarket proxy, 2 = Polymarket proxy + split).
    #[serde(rename = "signatureType")]
    pub signature_type: u8,
}

/// CLOB API order submission request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateOrderRequest {
    pub order: SignedOrder,
    pub owner: String,
    #[serde(rename = "orderType")]
    pub order_type: PolymarketOrderType,
}

/// CLOB API order submission response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateOrderResponse {
    pub success: bool,
    #[serde(rename = "orderID")]
    pub order_id: Option<String>,
    #[serde(rename = "errorMsg")]
    pub error_msg: Option<String>,
    pub status: Option<String>,
}

/// CLOB API cancel order request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelOrderRequest {
    #[serde(rename = "orderID")]
    pub order_id: String,
}

/// CLOB API cancel order response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelOrderResponse {
    pub success: bool,
    #[serde(rename = "errorMsg")]
    pub error_msg: Option<String>,
}

/// Order from CLOB API get-order endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolymarketOrder {
    pub id: String,
    pub status: PolymarketOrderStatus,
    pub side: PolymarketSide,
    pub price: String,
    #[serde(rename = "originalSize")]
    pub original_size: String,
    #[serde(rename = "remainingSize")]
    pub remaining_size: String,
    #[serde(rename = "tokenID")]
    pub token_id: String,
    #[serde(rename = "createdAt")]
    pub created_at: Option<String>,
}

/// Trade (fill) from WebSocket USER channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolymarketTrade {
    pub id: String,
    #[serde(rename = "orderID")]
    pub order_id: String,
    pub side: PolymarketSide,
    pub price: String,
    pub size: String,
    #[serde(rename = "tokenID")]
    pub token_id: String,
    pub timestamp: String,
    #[serde(rename = "matchOrderID")]
    pub match_order_id: Option<String>,
    pub fee: Option<String>,
}

/// WebSocket message from Polymarket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsMessage {
    #[serde(rename = "order")]
    OrderUpdate(PolymarketOrder),
    #[serde(rename = "trade")]
    TradeUpdate(PolymarketTrade),
    #[serde(rename = "ping")]
    Ping,
}

/// Account position from Polymarket positions API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolymarketPosition {
    #[serde(rename = "tokenID")]
    pub token_id: String,
    pub size: String,
    #[serde(rename = "avgPrice")]
    pub avg_price: String,
    #[serde(rename = "unrealizedPnl")]
    pub unrealized_pnl: Option<String>,
}
