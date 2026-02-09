//! WebSocket connection management for Polymarket real-time updates.
//!
//! Connects to the Polymarket WebSocket API for real-time order and trade
//! updates on the USER channel. Handles connection lifecycle, reconnection,
//! and message parsing.

use std::time::Duration;

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::types::WsMessage;

/// WebSocket connection state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsState {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
}

/// Reconnection configuration.
#[derive(Debug, Clone)]
pub struct ReconnectConfig {
    /// Initial delay between reconnection attempts.
    pub initial_delay: Duration,
    /// Maximum delay between reconnection attempts.
    pub max_delay: Duration,
    /// Backoff multiplier.
    pub backoff_factor: f64,
    /// Maximum number of reconnection attempts (0 = unlimited).
    pub max_attempts: u32,
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            backoff_factor: 2.0,
            max_attempts: 0, // Unlimited
        }
    }
}

/// WebSocket authentication credentials (L2 API creds).
#[derive(Debug, Clone, Serialize)]
struct AuthCreds {
    #[serde(rename = "apiKey")]
    api_key: String,
    secret: String,
    passphrase: String,
}

/// WebSocket authentication message.
#[derive(Debug, Clone, Serialize)]
struct AuthMessage {
    auth: AuthCreds,
    #[serde(rename = "type")]
    msg_type: String,
}

/// WebSocket subscription message.
#[derive(Debug, Clone, Serialize)]
struct SubscribeMessage {
    assets: Vec<String>,
    #[serde(rename = "type")]
    msg_type: String,
}

/// WebSocket pong response.
#[derive(Debug, Clone, Serialize)]
struct PongMessage {
    #[serde(rename = "type")]
    msg_type: String,
}

/// Raw WebSocket message from Polymarket.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RawWsMessage {
    /// Order update message.
    Order(OrderMessage),
    /// Trade update message.
    Trade(TradeMessage),
    /// Ping message.
    Ping(PingMessage),
    /// Error message.
    Error(ErrorMessage),
    /// Generic JSON message (fallback).
    Generic(serde_json::Value),
}

/// Order update from WebSocket.
#[derive(Debug, Clone, Deserialize)]
pub struct OrderMessage {
    #[serde(rename = "type")]
    pub msg_type: Option<String>,
    pub event_type: Option<String>,
    pub order_id: Option<String>,
    pub market: Option<String>,
    pub asset_id: Option<String>,
    pub side: Option<String>,
    pub price: Option<String>,
    pub original_size: Option<String>,
    pub size_matched: Option<String>,
    pub status: Option<String>,
    pub timestamp: Option<String>,
}

/// Trade update from WebSocket.
#[derive(Debug, Clone, Deserialize)]
pub struct TradeMessage {
    #[serde(rename = "type")]
    pub msg_type: Option<String>,
    pub id: Option<String>,
    pub taker_order_id: Option<String>,
    pub market: Option<String>,
    pub asset_id: Option<String>,
    pub side: Option<String>,
    pub price: Option<String>,
    pub size: Option<String>,
    pub fee_rate_bps: Option<String>,
    pub status: Option<String>,
    pub match_time: Option<String>,
    pub last_update: Option<String>,
    pub maker_orders: Option<Vec<MakerOrder>>,
}

/// Maker order within a trade.
#[derive(Debug, Clone, Deserialize)]
pub struct MakerOrder {
    pub order_id: Option<String>,
    pub asset_id: Option<String>,
    pub matched_amount: Option<String>,
    pub price: Option<String>,
    pub fee_rate_bps: Option<String>,
}

/// Ping message.
#[derive(Debug, Clone, Deserialize)]
pub struct PingMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
}

/// Error message.
#[derive(Debug, Clone, Deserialize)]
pub struct ErrorMessage {
    #[serde(rename = "type")]
    pub msg_type: Option<String>,
    pub error: Option<String>,
    pub message: Option<String>,
}

/// WebSocket connection manager for Polymarket.
#[derive(Debug)]
pub struct PolymarketWebSocket {
    url: String,
    state: WsState,
    api_key: String,
    api_secret: String,
    api_passphrase: String,
    reconnect_config: ReconnectConfig,
    subscribed_assets: Vec<String>,
    shutdown_tx: Option<mpsc::Sender<()>>,
}

impl PolymarketWebSocket {
    /// Creates a new WebSocket manager.
    #[must_use]
    pub fn new(url: String, api_key: String, api_secret: String, api_passphrase: String) -> Self {
        Self {
            url,
            state: WsState::Disconnected,
            api_key,
            api_secret,
            api_passphrase,
            reconnect_config: ReconnectConfig::default(),
            subscribed_assets: Vec::new(),
            shutdown_tx: None,
        }
    }

    /// Set the reconnection configuration.
    #[must_use]
    pub fn with_reconnect_config(mut self, config: ReconnectConfig) -> Self {
        self.reconnect_config = config;
        self
    }

    /// Add an asset (condition_id) to subscribe to.
    pub fn add_subscription(&mut self, asset_id: String) {
        if !self.subscribed_assets.contains(&asset_id) {
            self.subscribed_assets.push(asset_id);
        }
    }

    /// Check if there are any subscriptions.
    #[must_use]
    pub fn has_subscriptions(&self) -> bool {
        !self.subscribed_assets.is_empty()
    }

    /// Connect to the WebSocket and subscribe to the USER channel.
    ///
    /// Returns a receiver for incoming messages.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection or subscription fails.
    pub async fn connect(&mut self) -> Result<mpsc::UnboundedReceiver<WsMessage>> {
        self.state = WsState::Connecting;
        log::info!("Polymarket WebSocket: connecting to {}", self.url);

        // Connect to WebSocket (pass URL as string, tokio-tungstenite handles parsing)
        let (ws_stream, _response) = connect_async(&self.url).await.map_err(|e| {
            log::error!("Polymarket WebSocket: connection failed: {}", e);
            self.state = WsState::Disconnected;
            anyhow::anyhow!("WebSocket connection failed: {}", e)
        })?;

        log::info!("Polymarket WebSocket: connected");

        let (mut write, read) = ws_stream.split();

        // Send authentication message with full L2 credentials
        let auth_msg = AuthMessage {
            auth: AuthCreds {
                api_key: self.api_key.clone(),
                secret: self.api_secret.clone(),
                passphrase: self.api_passphrase.clone(),
            },
            msg_type: "user".to_string(),
        };
        let auth_json = serde_json::to_string(&auth_msg)?;
        write.send(Message::Text(auth_json.into())).await.map_err(|e| {
            log::error!("Polymarket WebSocket: auth send failed: {}", e);
            anyhow::anyhow!("WebSocket auth failed: {}", e)
        })?;
        log::info!("Polymarket WebSocket: auth message sent");

        // Send subscription message if we have assets
        if !self.subscribed_assets.is_empty() {
            let sub_msg = SubscribeMessage {
                assets: self.subscribed_assets.clone(),
                msg_type: "subscribe".to_string(),
            };
            let sub_json = serde_json::to_string(&sub_msg)?;
            write.send(Message::Text(sub_json.into())).await.map_err(|e| {
                log::error!("Polymarket WebSocket: subscribe send failed: {}", e);
                anyhow::anyhow!("WebSocket subscribe failed: {}", e)
            })?;
            log::info!(
                "Polymarket WebSocket: subscribed to {} assets",
                self.subscribed_assets.len()
            );
        }

        self.state = WsState::Connected;

        // Create channels for messages and shutdown
        let (tx, rx) = mpsc::unbounded_channel();
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);

        // Spawn task to read messages
        let url_clone = self.url.clone();
        let auth_creds = AuthCreds {
            api_key: self.api_key.clone(),
            secret: self.api_secret.clone(),
            passphrase: self.api_passphrase.clone(),
        };
        let assets_clone = self.subscribed_assets.clone();
        let reconnect_config = self.reconnect_config.clone();

        tokio::spawn(async move {
            let mut current_delay = reconnect_config.initial_delay;
            let mut reconnect_attempts = 0u32;
            let mut write = write;
            let mut read = read;
            let mut keepalive = tokio::time::interval(Duration::from_secs(30));
            keepalive.tick().await; // consume the immediate first tick

            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        log::info!("Polymarket WebSocket: shutdown signal received");
                        break;
                    }
                    _ = keepalive.tick() => {
                        // Use WebSocket protocol-level PING frame (not JSON
                        // {"type":"ping"} which the /ws/user channel rejects
                        // as "INVALID OPERATION").
                        if let Err(e) = write.send(Message::Ping(vec![].into())).await {
                            log::warn!("Polymarket WebSocket: keepalive ping failed: {}", e);
                        }
                    }
                    msg = read.next() => {
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                // Reset reconnect state on successful message
                                current_delay = reconnect_config.initial_delay;
                                reconnect_attempts = 0;

                                // Parse and handle message
                                match serde_json::from_str::<RawWsMessage>(&text) {
                                    Ok(raw_msg) => {
                                        match raw_msg {
                                            RawWsMessage::Ping(_) => {
                                                // Send pong response
                                                let pong = PongMessage { msg_type: "pong".to_string() };
                                                if let Ok(pong_json) = serde_json::to_string(&pong) {
                                                    let _ = write.send(Message::Text(pong_json.into())).await;
                                                }
                                            }
                                            RawWsMessage::Order(order) => {
                                                log::debug!("Polymarket WebSocket: order update: {:?}", order);
                                                // Convert to WsMessage::OrderUpdate if possible
                                                if let Some(ws_msg) = convert_order_to_ws_message(&order) {
                                                    let _ = tx.send(ws_msg);
                                                }
                                            }
                                            RawWsMessage::Trade(trade) => {
                                                log::debug!("Polymarket WebSocket: trade update: {:?}", trade);
                                                // Convert to WsMessage::TradeUpdate if possible
                                                if let Some(ws_msg) = convert_trade_to_ws_message(&trade) {
                                                    let _ = tx.send(ws_msg);
                                                }
                                            }
                                            RawWsMessage::Error(err) => {
                                                log::error!(
                                                    "Polymarket WebSocket: error: {:?} - {:?}",
                                                    err.error,
                                                    err.message
                                                );
                                            }
                                            RawWsMessage::Generic(value) => {
                                                log::debug!("Polymarket WebSocket: generic message: {}", value);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        log::warn!(
                                            "Polymarket WebSocket: failed to parse message: {} - raw: {}",
                                            e,
                                            text
                                        );
                                    }
                                }
                            }
                            Some(Ok(Message::Ping(data))) => {
                                let _ = write.send(Message::Pong(data)).await;
                            }
                            Some(Ok(Message::Close(frame))) => {
                                log::warn!("Polymarket WebSocket: connection closed: {:?}", frame);
                                // Attempt reconnection
                                if reconnect_config.max_attempts == 0
                                    || reconnect_attempts < reconnect_config.max_attempts
                                {
                                    reconnect_attempts += 1;
                                    log::info!(
                                        "Polymarket WebSocket: reconnecting in {:?} (attempt {})",
                                        current_delay,
                                        reconnect_attempts
                                    );
                                    tokio::time::sleep(current_delay).await;

                                    // Exponential backoff
                                    current_delay = Duration::from_secs_f64(
                                        (current_delay.as_secs_f64() * reconnect_config.backoff_factor)
                                            .min(reconnect_config.max_delay.as_secs_f64()),
                                    );

                                    // Attempt reconnection
                                    match reconnect(&url_clone, &auth_creds, &assets_clone).await {
                                        Ok((new_write, new_read)) => {
                                            write = new_write;
                                            read = new_read;
                                            log::info!("Polymarket WebSocket: reconnected successfully");
                                        }
                                        Err(e) => {
                                            log::error!("Polymarket WebSocket: reconnection failed: {}", e);
                                        }
                                    }
                                } else {
                                    log::error!(
                                        "Polymarket WebSocket: max reconnection attempts ({}) reached",
                                        reconnect_config.max_attempts
                                    );
                                    break;
                                }
                            }
                            Some(Err(e)) => {
                                log::error!("Polymarket WebSocket: error: {}", e);
                                // Attempt reconnection on error
                                if reconnect_config.max_attempts == 0
                                    || reconnect_attempts < reconnect_config.max_attempts
                                {
                                    reconnect_attempts += 1;
                                    log::info!(
                                        "Polymarket WebSocket: reconnecting after error in {:?} (attempt {})",
                                        current_delay,
                                        reconnect_attempts
                                    );
                                    tokio::time::sleep(current_delay).await;

                                    // Exponential backoff
                                    current_delay = Duration::from_secs_f64(
                                        (current_delay.as_secs_f64() * reconnect_config.backoff_factor)
                                            .min(reconnect_config.max_delay.as_secs_f64()),
                                    );

                                    // Attempt reconnection
                                    match reconnect(&url_clone, &auth_creds, &assets_clone).await {
                                        Ok((new_write, new_read)) => {
                                            write = new_write;
                                            read = new_read;
                                            current_delay = reconnect_config.initial_delay;
                                            reconnect_attempts = 0;
                                            log::info!("Polymarket WebSocket: reconnected successfully after error");
                                        }
                                        Err(reconnect_err) => {
                                            log::error!("Polymarket WebSocket: reconnection failed: {}", reconnect_err);
                                            // Continue loop to retry
                                        }
                                    }
                                } else {
                                    log::error!(
                                        "Polymarket WebSocket: max reconnection attempts ({}) reached after error",
                                        reconnect_config.max_attempts
                                    );
                                    break;
                                }
                            }
                            None => {
                                log::warn!("Polymarket WebSocket: stream ended unexpectedly");
                                // Attempt reconnection when stream ends
                                if reconnect_config.max_attempts == 0
                                    || reconnect_attempts < reconnect_config.max_attempts
                                {
                                    reconnect_attempts += 1;
                                    log::info!(
                                        "Polymarket WebSocket: reconnecting after stream end in {:?} (attempt {})",
                                        current_delay,
                                        reconnect_attempts
                                    );
                                    tokio::time::sleep(current_delay).await;

                                    // Exponential backoff
                                    current_delay = Duration::from_secs_f64(
                                        (current_delay.as_secs_f64() * reconnect_config.backoff_factor)
                                            .min(reconnect_config.max_delay.as_secs_f64()),
                                    );

                                    // Attempt reconnection
                                    match reconnect(&url_clone, &auth_creds, &assets_clone).await {
                                        Ok((new_write, new_read)) => {
                                            write = new_write;
                                            read = new_read;
                                            current_delay = reconnect_config.initial_delay;
                                            reconnect_attempts = 0;
                                            log::info!("Polymarket WebSocket: reconnected successfully after stream end");
                                        }
                                        Err(reconnect_err) => {
                                            log::error!("Polymarket WebSocket: reconnection failed: {}", reconnect_err);
                                            // Continue loop to retry
                                        }
                                    }
                                } else {
                                    log::error!(
                                        "Polymarket WebSocket: max reconnection attempts ({}) reached after stream end",
                                        reconnect_config.max_attempts
                                    );
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }

            log::info!("Polymarket WebSocket: reader task ended");
        });

        Ok(rx)
    }

    /// Disconnect from the WebSocket.
    pub async fn disconnect(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(()).await;
        }
        self.state = WsState::Disconnected;
        log::info!("Polymarket WebSocket: disconnected");
    }

    /// Get the current connection state.
    #[must_use]
    pub fn state(&self) -> &WsState {
        &self.state
    }
}

/// Reconnect to WebSocket with authentication.
async fn reconnect(
    url: &str,
    creds: &AuthCreds,
    assets: &[String],
) -> Result<(
    futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
)> {
    // Connect directly with string URL (tokio-tungstenite handles parsing)
    let (ws_stream, _) = connect_async(url).await?;
    let (mut write, read) = ws_stream.split();

    // Send auth with full L2 credentials
    let auth_msg = AuthMessage {
        auth: creds.clone(),
        msg_type: "user".to_string(),
    };
    let auth_json = serde_json::to_string(&auth_msg)?;
    write.send(Message::Text(auth_json.into())).await?;

    // Send subscription
    if !assets.is_empty() {
        let sub_msg = SubscribeMessage {
            assets: assets.to_vec(),
            msg_type: "subscribe".to_string(),
        };
        let sub_json = serde_json::to_string(&sub_msg)?;
        write.send(Message::Text(sub_json.into())).await?;
    }

    Ok((write, read))
}

/// Convert an order message to WsMessage.
fn convert_order_to_ws_message(order: &OrderMessage) -> Option<WsMessage> {
    use crate::types::{PolymarketOrder, PolymarketOrderStatus, PolymarketSide};

    let id = order.order_id.clone()?;
    let status = match order.status.as_deref() {
        Some("LIVE") | Some("live") => PolymarketOrderStatus::Live,
        Some("MATCHED") | Some("matched") => PolymarketOrderStatus::Matched,
        Some("CANCELLED") | Some("cancelled") | Some("CANCELED") | Some("canceled") => {
            PolymarketOrderStatus::Cancelled
        }
        Some("EXPIRED") | Some("expired") => PolymarketOrderStatus::Expired,
        _ => return None,
    };
    let side = match order.side.as_deref() {
        Some("BUY") | Some("buy") => PolymarketSide::Buy,
        Some("SELL") | Some("sell") => PolymarketSide::Sell,
        _ => return None,
    };

    Some(WsMessage::OrderUpdate(PolymarketOrder {
        id,
        status,
        side,
        price: order.price.clone().unwrap_or_default(),
        original_size: order.original_size.clone().unwrap_or_default(),
        remaining_size: order
            .size_matched
            .as_ref()
            .map(|matched| {
                let orig: f64 = order
                    .original_size
                    .as_ref()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0.0);
                let m: f64 = matched.parse().unwrap_or(0.0);
                (orig - m).to_string()
            })
            .unwrap_or_else(|| order.original_size.clone().unwrap_or_default()),
        token_id: order.asset_id.clone().unwrap_or_default(),
        created_at: order.timestamp.clone(),
    }))
}

/// Convert a trade message to WsMessage.
fn convert_trade_to_ws_message(trade: &TradeMessage) -> Option<WsMessage> {
    use crate::types::{PolymarketSide, PolymarketTrade};

    let id = trade.id.clone()?;
    let order_id = trade.taker_order_id.clone()?;
    let side = match trade.side.as_deref() {
        Some("BUY") | Some("buy") => PolymarketSide::Buy,
        Some("SELL") | Some("sell") => PolymarketSide::Sell,
        _ => return None,
    };

    Some(WsMessage::TradeUpdate(PolymarketTrade {
        id,
        order_id,
        side,
        price: trade.price.clone().unwrap_or_default(),
        size: trade.size.clone().unwrap_or_default(),
        token_id: trade.asset_id.clone().unwrap_or_default(),
        timestamp: trade.match_time.clone().unwrap_or_default(),
        match_order_id: trade
            .maker_orders
            .as_ref()
            .and_then(|orders| orders.first())
            .and_then(|o| o.order_id.clone()),
        fee: trade
            .maker_orders
            .as_ref()
            .and_then(|orders| orders.first())
            .and_then(|o| o.fee_rate_bps.clone()),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ws_state() {
        let ws = PolymarketWebSocket::new(
            "wss://example.com/ws".to_string(),
            "test_key".to_string(),
            "test_secret".to_string(),
            "test_pass".to_string(),
        );
        assert_eq!(ws.state(), &WsState::Disconnected);
    }

    #[test]
    fn test_add_subscription() {
        let mut ws = PolymarketWebSocket::new(
            "wss://example.com/ws".to_string(),
            "test_key".to_string(),
            "test_secret".to_string(),
            "test_pass".to_string(),
        );
        assert!(!ws.has_subscriptions());

        ws.add_subscription("asset1".to_string());
        assert!(ws.has_subscriptions());

        ws.add_subscription("asset1".to_string()); // Duplicate
        assert_eq!(ws.subscribed_assets.len(), 1);

        ws.add_subscription("asset2".to_string());
        assert_eq!(ws.subscribed_assets.len(), 2);
    }

    #[test]
    fn test_reconnect_config() {
        let config = ReconnectConfig::default();
        assert_eq!(config.initial_delay, Duration::from_secs(1));
        assert_eq!(config.max_delay, Duration::from_secs(60));
        assert_eq!(config.backoff_factor, 2.0);
        assert_eq!(config.max_attempts, 0);
    }

    #[test]
    fn test_auth_message_serialization() {
        let auth = AuthMessage {
            auth: AuthCreds {
                api_key: "test_key".to_string(),
                secret: "test_secret".to_string(),
                passphrase: "test_pass".to_string(),
            },
            msg_type: "user".to_string(),
        };
        let json = serde_json::to_string(&auth).unwrap();
        assert!(json.contains("\"apiKey\":\"test_key\""));
        assert!(json.contains("\"secret\":\"test_secret\""));
        assert!(json.contains("\"passphrase\":\"test_pass\""));
        assert!(json.contains("\"type\":\"user\""));
    }
}
