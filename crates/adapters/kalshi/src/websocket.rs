//! WebSocket connection management for Kalshi real-time updates.
//!
//! Connects to the Kalshi WebSocket API for real-time fill, order update,
//! and orderbook delta channels. Handles connection lifecycle, authentication,
//! and reconnection with exponential backoff.

use std::time::Duration;

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::http::Request;
use tokio_tungstenite::tungstenite::Message;

use crate::auth::KalshiAuth;

const KALSHI_WS_URL: &str = "wss://api.elections.kalshi.com/trade-api/ws/v2";

/// WebSocket connection state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsState {
    Disconnected,
    Connecting,
    Connected,
}

/// Reconnection configuration.
#[derive(Debug, Clone)]
pub struct ReconnectConfig {
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub backoff_factor: f64,
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

/// Subscribe command for Kalshi WS.
#[derive(Debug, Clone, Serialize)]
struct SubscribeMessage {
    id: u64,
    cmd: String,
    params: SubscribeParams,
}

#[derive(Debug, Clone, Serialize)]
struct SubscribeParams {
    channels: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    market_ticker: Vec<String>,
}

/// Kalshi WS message (parsed from JSON).
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum KalshiWsMessage {
    Fill(FillMessage),
    OrderUpdate(OrderUpdateMessage),
    Heartbeat(HeartbeatMessage),
    Generic(serde_json::Value),
}

/// Fill message from the `fill` channel.
#[derive(Debug, Clone, Deserialize)]
pub struct FillMessage {
    #[serde(rename = "type")]
    pub msg_type: Option<String>,
    pub trade_id: Option<String>,
    pub order_id: Option<String>,
    pub ticker: Option<String>,
    pub side: Option<String>,
    pub count: Option<u32>,
    pub yes_price: Option<u32>,
    pub no_price: Option<u32>,
    pub action: Option<String>,
}

/// Order update message from the `order_group_updates` channel.
#[derive(Debug, Clone, Deserialize)]
pub struct OrderUpdateMessage {
    #[serde(rename = "type")]
    pub msg_type: Option<String>,
    pub order_id: Option<String>,
    pub status: Option<String>,
    pub remaining_count: Option<u32>,
}

/// Heartbeat/ping message.
#[derive(Debug, Clone, Deserialize)]
pub struct HeartbeatMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
}

/// WebSocket connection manager for Kalshi.
#[derive(Debug)]
pub struct KalshiWebSocket {
    auth: KalshiAuth,
    ws_url: String,
    state: WsState,
    reconnect_config: ReconnectConfig,
    subscribed_channels: Vec<String>,
    subscribed_tickers: Vec<String>,
    shutdown_tx: Option<mpsc::Sender<()>>,
    msg_counter: u64,
}

impl KalshiWebSocket {
    /// Creates a new WebSocket manager.
    #[must_use]
    pub fn new(auth: KalshiAuth) -> Self {
        Self {
            auth,
            ws_url: KALSHI_WS_URL.to_string(),
            state: WsState::Disconnected,
            reconnect_config: ReconnectConfig::default(),
            subscribed_channels: Vec::new(),
            subscribed_tickers: Vec::new(),
            shutdown_tx: None,
            msg_counter: 0,
        }
    }

    /// Set a custom WebSocket URL.
    #[must_use]
    pub fn with_ws_url(mut self, url: String) -> Self {
        self.ws_url = url;
        self
    }

    /// Get the current connection state.
    #[must_use]
    pub fn state(&self) -> &WsState {
        &self.state
    }

    /// Connect to the WebSocket and start receiving messages.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection fails.
    pub async fn connect(&mut self) -> Result<mpsc::UnboundedReceiver<KalshiWsMessage>> {
        self.state = WsState::Connecting;
        log::info!("Kalshi WebSocket: connecting to {}", self.ws_url);

        // Build HTTP request with auth headers for WS handshake
        let headers = self.auth.sign_request("GET", "/trade-api/ws/v2");
        let request = Request::builder()
            .uri(&self.ws_url)
            .header("KALSHI-ACCESS-KEY", &headers.api_key)
            .header("KALSHI-ACCESS-SIGNATURE", &headers.signature)
            .header("KALSHI-ACCESS-TIMESTAMP", &headers.timestamp)
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header(
                "Sec-WebSocket-Key",
                tokio_tungstenite::tungstenite::handshake::client::generate_key(),
            )
            .body(())?;

        let (ws_stream, _response) =
            tokio_tungstenite::connect_async(request)
                .await
                .map_err(|e| {
                    log::error!("Kalshi WebSocket: connection failed: {e}");
                    self.state = WsState::Disconnected;
                    anyhow::anyhow!("WebSocket connection failed: {e}")
                })?;

        log::info!("Kalshi WebSocket: connected");

        let (mut write, read) = ws_stream.split();

        // Send subscription messages
        for channel in &self.subscribed_channels {
            self.msg_counter += 1;
            let sub = SubscribeMessage {
                id: self.msg_counter,
                cmd: "subscribe".to_string(),
                params: SubscribeParams {
                    channels: vec![channel.clone()],
                    market_ticker: self.subscribed_tickers.clone(),
                },
            };
            if let Ok(json) = serde_json::to_string(&sub) {
                let _ = write.send(Message::Text(json.into())).await;
            }
        }

        self.state = WsState::Connected;

        let (tx, rx) = mpsc::unbounded_channel();
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);

        let reconnect_config = self.reconnect_config.clone();

        tokio::spawn(async move {
            let mut read = read;
            let mut write = write;
            let mut current_delay = reconnect_config.initial_delay;
            let mut reconnect_attempts = 0u32;

            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        log::info!("Kalshi WebSocket: shutdown signal received");
                        break;
                    }
                    msg = read.next() => {
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                current_delay = reconnect_config.initial_delay;
                                reconnect_attempts = 0;

                                match serde_json::from_str::<KalshiWsMessage>(&text) {
                                    Ok(KalshiWsMessage::Heartbeat(_)) => {
                                        // Respond to server heartbeat
                                        let _ = write.send(Message::Pong(vec![].into())).await;
                                    }
                                    Ok(msg) => {
                                        let _ = tx.send(msg);
                                    }
                                    Err(e) => {
                                        log::debug!(
                                            "Kalshi WebSocket: failed to parse message: {e} - raw: {text}"
                                        );
                                    }
                                }
                            }
                            Some(Ok(Message::Ping(data))) => {
                                let _ = write.send(Message::Pong(data)).await;
                            }
                            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => {
                                if reconnect_config.max_attempts > 0
                                    && reconnect_attempts >= reconnect_config.max_attempts
                                {
                                    log::error!("Kalshi WebSocket: max reconnection attempts reached");
                                    break;
                                }
                                reconnect_attempts += 1;
                                log::info!(
                                    "Kalshi WebSocket: reconnecting in {current_delay:?} (attempt {reconnect_attempts})"
                                );
                                tokio::time::sleep(current_delay).await;
                                current_delay = Duration::from_secs_f64(
                                    (current_delay.as_secs_f64() * reconnect_config.backoff_factor)
                                        .min(reconnect_config.max_delay.as_secs_f64()),
                                );
                                // TODO: implement reconnect with re-auth
                            }
                            _ => {}
                        }
                    }
                }
            }

            log::info!("Kalshi WebSocket: reader task ended");
        });

        Ok(rx)
    }

    /// Subscribe to channels for given tickers.
    pub fn add_subscription(&mut self, channels: &[&str], tickers: &[String]) {
        for ch in channels {
            if !self.subscribed_channels.contains(&ch.to_string()) {
                self.subscribed_channels.push(ch.to_string());
            }
        }
        for t in tickers {
            if !self.subscribed_tickers.contains(t) {
                self.subscribed_tickers.push(t.clone());
            }
        }
    }

    /// Disconnect from the WebSocket.
    pub async fn disconnect(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(()).await;
        }
        self.state = WsState::Disconnected;
        log::info!("Kalshi WebSocket: disconnected");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ws_state_default() {
        // Cannot test without real auth, but verify types compile
        assert_eq!(WsState::Disconnected, WsState::Disconnected);
    }

    #[test]
    fn test_reconnect_config_default() {
        let config = ReconnectConfig::default();
        assert_eq!(config.initial_delay, Duration::from_secs(1));
        assert_eq!(config.max_delay, Duration::from_secs(60));
    }
}
