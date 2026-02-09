//! Kalshi execution client implementing the NautilusTrader `ExecutionClient` trait.
//!
//! Handles:
//! - HTTP: submit_order, cancel_order via Kalshi REST API
//! - WS: real-time fill/order updates via `fill` and `order_group_updates` channels
//! - Order state: pending → accepted → filled/canceled
//! - RSA-PSS authenticated requests

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use async_trait::async_trait;
use nautilus_common::cache::Cache;
use nautilus_common::clients::ExecutionClient;
use nautilus_common::messages::execution::{CancelOrder, ModifyOrder, SubmitOrder};
use nautilus_core::UnixNanos;
use nautilus_model::{
    accounts::AccountAny,
    enums::{OmsType, OrderType, TimeInForce},
    identifiers::{AccountId, ClientId, ClientOrderId, Venue, VenueOrderId},
    orders::Order,
    types::{AccountBalance, Currency, MarginBalance, Money},
};

use crate::auth::{apply_auth, KalshiAuth};
use crate::config::{KalshiExecutionClientConfig, TickerMap};
use crate::types::{BalanceResponse, CreateOrderRequest, OrderResponse};
use crate::websocket::KalshiWebSocket;

/// Kalshi execution client.
///
/// Implements the NautilusTrader `ExecutionClient` trait for real order
/// execution on the Kalshi CLOB.
#[derive(Debug)]
pub struct KalshiExecutionClient {
    client_id: ClientId,
    venue: Venue,
    account_id: AccountId,
    config: KalshiExecutionClientConfig,
    cache: Rc<RefCell<Cache>>,
    http_client: reqwest::Client,
    auth: KalshiAuth,
    ws: Option<KalshiWebSocket>,
    order_id_map: HashMap<ClientOrderId, VenueOrderId>,
    is_connected: bool,
    ticker_map: Option<TickerMap>,
    order_results: Option<crate::config::OrderResultMap>,
}

impl KalshiExecutionClient {
    /// Creates a new `KalshiExecutionClient`.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client cannot be created or RSA key is invalid.
    pub fn new(
        client_id: ClientId,
        config: KalshiExecutionClientConfig,
        cache: Rc<RefCell<Cache>>,
    ) -> anyhow::Result<Self> {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;

        let auth = KalshiAuth::new(config.api_key.clone(), &config.private_key_pem)?;
        log::info!(
            "KalshiExecutionClient: api_key={}... asset={} subaccount={:?}",
            &config.api_key[..8.min(config.api_key.len())],
            config.asset,
            config.subaccount,
        );

        let ticker_map = config.ticker_map.clone();
        let order_results = config.order_results.clone();

        Ok(Self {
            client_id,
            venue: config.venue,
            account_id: config.account_id,
            config,
            cache,
            http_client,
            auth,
            ws: None,
            order_id_map: HashMap::new(),
            is_connected: false,
            ticker_map,
            order_results,
        })
    }

    /// Get an order from the cache by client order ID.
    fn get_order(
        &self,
        client_order_id: &ClientOrderId,
    ) -> anyhow::Result<nautilus_model::orders::OrderAny> {
        self.cache
            .borrow()
            .order(client_order_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Order not found in cache: {client_order_id}"))
    }

    /// Extract Kalshi market ticker from instrument ID via the shared ticker map.
    fn resolve_ticker(
        instrument_id: &nautilus_model::identifiers::InstrumentId,
        ticker_map: &Option<TickerMap>,
    ) -> anyhow::Result<String> {
        let symbol = instrument_id.symbol.as_str();

        if let Some(map) = ticker_map {
            if let Ok(guard) = map.read() {
                if let Some(ticker) = guard.get(symbol) {
                    log::info!(
                        "resolve_ticker: symbol={symbol} → ticker={ticker}",
                    );
                    return Ok(ticker.clone());
                }
            }
        }

        Err(anyhow::anyhow!(
            "No Kalshi ticker resolved for instrument {instrument_id}. \
             Ensure the ticker resolver is running and has mapped this instrument."
        ))
    }

    /// Determine Kalshi order side from instrument symbol.
    /// Instrument symbols containing "Up" map to "yes", "Down" to "no".
    fn resolve_side(instrument_id: &nautilus_model::identifiers::InstrumentId) -> &'static str {
        let symbol = instrument_id.symbol.as_str();
        if symbol.contains("Up") || symbol.contains("UP") || symbol.contains("up") {
            "yes"
        } else {
            "no"
        }
    }

    /// Update account state from balance API.
    async fn update_account_state(&self) -> anyhow::Result<AccountBalance> {
        let path = "/portfolio/balance";
        let headers = self.auth.sign_request("GET", path);
        let url = format!("{}{}", self.config.rest_url, path);

        let response = apply_auth(self.http_client.get(&url), &headers)
            .send()
            .await?;

        let status = response.status();
        let text = response.text().await?;

        if !status.is_success() {
            return Err(anyhow::anyhow!(
                "Kalshi balance query failed with status {status}: {text}"
            ));
        }

        let balance: BalanceResponse = serde_json::from_str(&text)?;
        let balance_dollars = balance.balance as f64 / 100.0;

        log::info!("Kalshi balance: {} cents = ${balance_dollars:.2}", balance.balance);

        let usd = Currency::from("USD");
        let total = Money::new(balance_dollars, usd);
        Ok(AccountBalance::new(total, Money::new(0.0, usd), total))
    }
}

#[async_trait(?Send)]
impl ExecutionClient for KalshiExecutionClient {
    fn is_connected(&self) -> bool {
        self.is_connected
    }

    fn client_id(&self) -> ClientId {
        self.client_id
    }

    fn account_id(&self) -> AccountId {
        self.account_id
    }

    fn venue(&self) -> Venue {
        self.venue
    }

    fn oms_type(&self) -> OmsType {
        OmsType::Netting
    }

    fn get_account(&self) -> Option<AccountAny> {
        None
    }

    fn generate_account_state(
        &self,
        _balances: Vec<AccountBalance>,
        _margins: Vec<MarginBalance>,
        _reported: bool,
        _ts_event: UnixNanos,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn start(&mut self) -> anyhow::Result<()> {
        log::info!("KalshiExecutionClient: starting...");
        self.is_connected = true;
        Ok(())
    }

    fn stop(&mut self) -> anyhow::Result<()> {
        log::info!("KalshiExecutionClient: stopping...");
        self.is_connected = false;
        Ok(())
    }

    async fn connect(&mut self) -> anyhow::Result<()> {
        log::info!("KalshiExecutionClient: connecting...");

        // Initialize WebSocket for fill/order updates
        let mut ws = KalshiWebSocket::new(self.auth.clone());
        ws.add_subscription(&["fill", "order_group_updates"], &[]);

        match ws.connect().await {
            Ok(_rx) => {
                log::info!("KalshiExecutionClient: WebSocket connected");
                self.ws = Some(ws);
            }
            Err(e) => {
                log::warn!("KalshiExecutionClient: WebSocket connection failed (non-fatal): {e}");
            }
        }

        // Load initial balance
        match self.update_account_state().await {
            Ok(balance) => {
                log::info!("KalshiExecutionClient: balance loaded: {balance:?}");
            }
            Err(e) => {
                log::warn!("KalshiExecutionClient: failed to load balance: {e}");
            }
        }

        self.is_connected = true;
        log::info!("KalshiExecutionClient: connected");
        Ok(())
    }

    async fn disconnect(&mut self) -> anyhow::Result<()> {
        log::info!("KalshiExecutionClient: disconnecting...");

        if let Some(ref mut ws) = self.ws {
            ws.disconnect().await;
        }
        self.ws = None;

        self.is_connected = false;
        Ok(())
    }

    fn submit_order(&self, cmd: &SubmitOrder) -> anyhow::Result<()> {
        let order = self.get_order(&cmd.client_order_id)?;

        log::info!(
            "KalshiExecutionClient: submit_order {:?}",
            order.client_order_id()
        );

        // Validate order type
        match order.order_type() {
            OrderType::Limit | OrderType::Market => {}
            other => {
                return Err(anyhow::anyhow!(
                    "Order type {other:?} not supported on Kalshi; use MARKET or LIMIT"
                ));
            }
        }

        // Resolve Kalshi market ticker from instrument
        let ticker = Self::resolve_ticker(&order.instrument_id(), &self.ticker_map)?;

        // Determine side from instrument symbol
        let kalshi_side = Self::resolve_side(&order.instrument_id());

        // Get price and quantity
        let price = order
            .price()
            .ok_or_else(|| anyhow::anyhow!("Kalshi limit order must have a price"))?;
        let price_cents = (price.as_f64() * 100.0).round() as u32;
        let count = order.quantity().as_f64() as u32;

        // Convert time-in-force
        let tif = match order.time_in_force() {
            TimeInForce::Gtc => None, // Kalshi default is GTC
            TimeInForce::Ioc => Some("ioc".to_string()),
            TimeInForce::Fok => Some("fok".to_string()),
            other => {
                log::warn!("Unsupported time-in-force {other:?}, defaulting to IOC");
                Some("ioc".to_string())
            }
        };

        let request = CreateOrderRequest {
            ticker: ticker.clone(),
            side: kalshi_side.to_string(),
            action: "buy".to_string(),
            count,
            order_type: "limit".to_string(),
            yes_price: if kalshi_side == "yes" {
                Some(price_cents)
            } else {
                None
            },
            no_price: if kalshi_side == "no" {
                Some(price_cents)
            } else {
                None
            },
            expiration_ts: None,
            time_in_force: tif,
            subaccount: self.config.subaccount,
        };

        // Clone values for async block
        let http_client = self.http_client.clone();
        let rest_url = self.config.rest_url.clone();
        let auth = self.auth.clone();
        let order_results = self.order_results.clone();
        let client_order_id = order.client_order_id();

        // Spawn async task to submit order
        tokio::spawn(async move {
            let path = "/portfolio/orders";
            let url = format!("{rest_url}{path}");
            let headers = auth.sign_request("POST", path);

            let body = match serde_json::to_string(&request) {
                Ok(b) => b,
                Err(e) => {
                    log::error!("Failed to serialize Kalshi order request: {e}");
                    return;
                }
            };

            log::info!("Kalshi order payload: {body}");

            let response = apply_auth(
                http_client
                    .post(&url)
                    .header("Content-Type", "application/json")
                    .body(body),
                &headers,
            )
            .send()
            .await;

            let write_result = |accepted: bool| {
                if let Some(ref map) = order_results {
                    if let Ok(mut guard) = map.write() {
                        guard.insert(client_order_id.to_string(), accepted);
                    }
                }
            };

            match response {
                Ok(resp) => {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();

                    if status.is_success() {
                        match serde_json::from_str::<OrderResponse>(&text) {
                            Ok(order_resp) => {
                                log::info!(
                                    "Kalshi order {:?} submitted: order_id={} status={} remaining={:?}",
                                    client_order_id,
                                    order_resp.order.order_id,
                                    order_resp.order.status,
                                    order_resp.order.remaining_count,
                                );
                                write_result(true);
                            }
                            Err(e) => {
                                log::error!(
                                    "Failed to parse Kalshi order response: {e} - body: {text}"
                                );
                                write_result(false);
                            }
                        }
                    } else {
                        log::error!(
                            "Kalshi order {:?} failed with status {status}: {text}",
                            client_order_id,
                        );
                        write_result(false);
                    }
                }
                Err(e) => {
                    log::error!("Kalshi order {:?} HTTP error: {e}", client_order_id);
                    write_result(false);
                }
            }
        });

        Ok(())
    }

    fn cancel_order(&self, cmd: &CancelOrder) -> anyhow::Result<()> {
        log::info!(
            "KalshiExecutionClient: cancel_order {:?}",
            cmd.client_order_id
        );

        let venue_order_id = match cmd.venue_order_id {
            Some(ref id) => *id,
            None => match self.order_id_map.get(&cmd.client_order_id) {
                Some(id) => *id,
                None => {
                    return Err(anyhow::anyhow!(
                        "Cannot cancel order {:?}: venue_order_id not found",
                        cmd.client_order_id
                    ));
                }
            },
        };

        let http_client = self.http_client.clone();
        let rest_url = self.config.rest_url.clone();
        let auth = self.auth.clone();
        let client_order_id = cmd.client_order_id;

        tokio::spawn(async move {
            let path = format!("/portfolio/orders/{}", venue_order_id.as_str());
            let url = format!("{rest_url}{path}");
            let headers = auth.sign_request("DELETE", &path);

            let response = apply_auth(http_client.delete(&url), &headers)
                .send()
                .await;

            match response {
                Ok(resp) => {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();

                    if status.is_success() {
                        log::info!("Kalshi order {:?} canceled successfully", client_order_id);
                    } else {
                        log::error!(
                            "Kalshi order {:?} cancel failed with status {status}: {text}",
                            client_order_id,
                        );
                    }
                }
                Err(e) => {
                    log::error!("Kalshi order {:?} cancel HTTP error: {e}", client_order_id);
                }
            }
        });

        Ok(())
    }

    fn modify_order(&self, cmd: &ModifyOrder) -> anyhow::Result<()> {
        log::info!(
            "KalshiExecutionClient: modify_order {:?} - not supported",
            cmd.client_order_id
        );
        anyhow::bail!("Order modification not supported by Kalshi. Cancel and resubmit.")
    }
}
