//! Polymarket execution client implementing the NautilusTrader `ExecutionClient` trait.
//!
//! Handles:
//! - HTTP: submit_order, cancel_order, get_order via CLOB REST API
//! - WS: real-time order/trade updates (USER channel)
//! - Order signing: EIP-712 for Polymarket's CTF Exchange
//! - Order state: pending → accepted → filled/canceled
//! - Account state from positions API

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use async_trait::async_trait;
use indexmap::IndexMap;
use nautilus_common::cache::Cache;
use nautilus_common::clients::ExecutionClient;
use nautilus_common::messages::execution::{CancelOrder, ModifyOrder, SubmitOrder};
use nautilus_core::UnixNanos;
use nautilus_model::{
    accounts::AccountAny,
    enums::{OmsType, OrderSide, OrderType, TimeInForce},
    identifiers::{AccountId, ClientId, ClientOrderId, TradeId, Venue, VenueOrderId},
    orders::{Order, OrderAny},
    types::{AccountBalance, Currency, MarginBalance, Money},
};
use tokio::sync::Notify;

use crate::config::{PolymarketExecutionClientConfig, TokenMap};
use crate::signing::{
    build_l2_headers, derive_address, generate_salt, sign_order, Eip712Domain, L2Headers,
    OrderData,
};
use crate::types::{
    CancelOrderResponse, CreateOrderRequest, CreateOrderResponse, PolymarketOrderType,
    PolymarketSide, SignedOrder,
};
use crate::websocket::PolymarketWebSocket;

/// Default taker fee rate in basis points (bps).
/// Polymarket requires orders to include the fee rate (1000 bps = 10% is standard).
const DEFAULT_FEE_RATE_BPS: u16 = 1000;

/// Polymarket trade status for deduplication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolymarketTradeStatus {
    Matched,
    Mined,
    Confirmed,
    Retrying,
    Failed,
}

/// Polymarket execution client.
///
/// Implements the NautilusTrader `ExecutionClient` trait for real order
/// execution on the Polymarket CLOB.
#[derive(Debug)]
pub struct PolymarketExecutionClient {
    client_id: ClientId,
    venue: Venue,
    account_id: AccountId,
    config: PolymarketExecutionClientConfig,
    cache: Rc<RefCell<Cache>>,
    http_client: reqwest::Client,
    ws: Option<PolymarketWebSocket>,
    /// Map from ClientOrderId to VenueOrderId (CLOB order ID).
    order_id_map: HashMap<ClientOrderId, VenueOrderId>,
    /// Derived wallet address from private key (used as signer).
    wallet_address: String,
    /// Address to use as `maker` in orders (funder_address or wallet_address).
    maker_address: String,
    /// Fill deduplication cache: (TradeId, VenueOrderId) -> processed.
    processed_fills: IndexMap<(TradeId, VenueOrderId), ()>,
    /// Trade status cache for deduplication.
    processed_trades: IndexMap<TradeId, PolymarketTradeStatus>,
    /// Finalized trades cache.
    finalized_trades: IndexMap<TradeId, ()>,
    /// Async event notifiers for order placement acks.
    ack_notifiers_order: HashMap<VenueOrderId, Arc<Notify>>,
    /// Async event notifiers for trade acks.
    ack_notifiers_trade: HashMap<VenueOrderId, Arc<Notify>>,
    is_connected: bool,
    /// Shared token map: instrument symbol → Polymarket CTF token ID.
    token_map: Option<TokenMap>,
    /// Shared order result map: client_order_id → accepted by CLOB.
    order_results: Option<crate::config::OrderResultMap>,
}

/// Maximum size for deduplication caches.
const PROCESSED_LIMIT: usize = 10_000;

impl PolymarketExecutionClient {
    /// Creates a new `PolymarketExecutionClient`.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client cannot be created or private key is invalid.
    pub fn new(
        client_id: ClientId,
        config: PolymarketExecutionClientConfig,
        cache: Rc<RefCell<Cache>>,
    ) -> anyhow::Result<Self> {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;

        // Derive wallet address from private key
        let wallet_address = derive_address(&config.private_key)?;
        let maker_address = config
            .funder_address
            .clone()
            .unwrap_or_else(|| wallet_address.clone());
        log::info!(
            "PolymarketExecutionClient: wallet_address={wallet_address} maker_address={maker_address} signature_type={}",
            config.signature_type
        );

        let token_map = config.token_map.clone();
        let order_results = config.order_results.clone();

        Ok(Self {
            client_id,
            venue: config.venue,
            account_id: config.account_id,
            config,
            cache,
            http_client,
            ws: None,
            order_id_map: HashMap::new(),
            wallet_address,
            maker_address,
            processed_fills: IndexMap::new(),
            processed_trades: IndexMap::new(),
            finalized_trades: IndexMap::new(),
            ack_notifiers_order: HashMap::new(),
            ack_notifiers_trade: HashMap::new(),
            is_connected: false,
            token_map,
            order_results,
        })
    }

    /// Get the EIP-712 domain for signing.
    fn eip712_domain(&self) -> Eip712Domain {
        Eip712Domain {
            name: "Polymarket CTF Exchange".to_string(),
            version: "1".to_string(),
            chain_id: self.config.chain_id,
            verifying_contract: self.config.ctf_exchange_address.clone(),
        }
    }

    /// Get an order from the cache by client order ID.
    fn get_order(&self, client_order_id: &ClientOrderId) -> anyhow::Result<OrderAny> {
        self.cache
            .borrow()
            .order(client_order_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Order not found in cache: {client_order_id}"))
    }

    /// Convert Nautilus order side to Polymarket side.
    fn convert_side(side: OrderSide) -> PolymarketSide {
        match side {
            OrderSide::Buy => PolymarketSide::Buy,
            OrderSide::Sell => PolymarketSide::Sell,
            _ => PolymarketSide::Buy, // Default, should not happen
        }
    }

    /// Convert Nautilus time-in-force to Polymarket order type.
    fn convert_tif(tif: TimeInForce) -> Option<PolymarketOrderType> {
        match tif {
            TimeInForce::Gtc => Some(PolymarketOrderType::Gtc),
            TimeInForce::Ioc => Some(PolymarketOrderType::Ioc),
            TimeInForce::Fok => Some(PolymarketOrderType::Fok),
            TimeInForce::Gtd => Some(PolymarketOrderType::Gtc), // GTD maps to GTC with expiration
            _ => None,
        }
    }

    /// Create a signed order for submission.
    fn create_signed_order(
        &self,
        order: &OrderAny,
        token_id: &str,
    ) -> anyhow::Result<SignedOrder> {
        let side = Self::convert_side(order.order_side());

        // Get price and quantity
        let price = order
            .price()
            .ok_or_else(|| anyhow::anyhow!("Order must have a price"))?;
        let quantity = order.quantity();

        // Convert to Polymarket amounts (USDC has 6 decimals, i.e. ×10^6).
        // CLOB precision constraints (tick_size=0.01):
        //   - size (token qty): max 2 decimal places → base units divisible by 10_000
        //   - amount (USDC):    max 4 decimal places → base units divisible by 100
        let price_f64 = price.as_f64();
        let qty_f64 = quantity.as_f64();

        // Round size down to 2 decimal places (matching py-clob-client round_down(size, 2))
        let qty_rounded = (qty_f64 * 100.0).floor() / 100.0;
        // Round price to 2 decimal places
        let price_rounded = (price_f64 * 100.0).round() / 100.0;

        // For BUY: maker gives USDC, receives tokens
        // For SELL: maker gives tokens, receives USDC
        let (maker_amount, taker_amount): (String, String) = match side {
            PolymarketSide::Buy => {
                let usdc_raw = price_rounded * qty_rounded;
                // Round USDC to 4 decimal places, then convert to base units
                let usdc_rounded = (usdc_raw * 10_000.0).round() / 10_000.0;
                let usdc_amount = (usdc_rounded * 1_000_000.0).round() as u64;
                let token_amount = (qty_rounded * 1_000_000.0).round() as u64;
                (usdc_amount.to_string(), token_amount.to_string())
            }
            PolymarketSide::Sell => {
                let usdc_raw = price_rounded * qty_rounded;
                let usdc_rounded = (usdc_raw * 10_000.0).round() / 10_000.0;
                let token_amount = (qty_rounded * 1_000_000.0).round() as u64;
                let usdc_amount = (usdc_rounded * 1_000_000.0).round() as u64;
                (token_amount.to_string(), usdc_amount.to_string())
            }
        };

        // Get expiration (0 = no expiration for GTC)
        let expiration = if order.time_in_force() == TimeInForce::Gtd {
            let expire_ns = order.expire_time().map(|t| t.as_u64()).unwrap_or(0);
            (expire_ns / 1_000_000_000).to_string() // Convert ns to seconds
        } else {
            "0".to_string()
        };

        let salt = generate_salt();
        let salt_str = salt.to_string();
        let nonce = "0".to_string(); // Nonce management handled by CLOB

        // Build order data for signing (uses string representation)
        // For sig_type=2 (Gnosis Safe), maker is the proxy wallet; signer is the EOA.
        let order_data = OrderData {
            salt: salt_str.clone(),
            maker: self.maker_address.clone(),
            signer: self.wallet_address.clone(),
            taker: OrderData::ZERO_ADDRESS.to_string(),
            token_id: token_id.to_string(),
            maker_amount: maker_amount.clone(),
            taker_amount: taker_amount.clone(),
            expiration: expiration.clone(),
            nonce: nonce.clone(),
            fee_rate_bps: DEFAULT_FEE_RATE_BPS.to_string(),
            side: match side {
                PolymarketSide::Buy => 0,
                PolymarketSide::Sell => 1,
            },
            signature_type: self.config.signature_type,
        };

        // Sign the order
        let domain = self.eip712_domain();
        let signature = sign_order(&self.config.private_key, &domain, &order_data)?;

        Ok(SignedOrder {
            signature,
            salt,
            maker: self.maker_address.clone(),
            signer: self.wallet_address.clone(),
            taker: OrderData::ZERO_ADDRESS.to_string(),
            token_id: token_id.to_string(),
            maker_amount,
            taker_amount,
            side,
            expiration,
            nonce,
            fee_rate_bps: DEFAULT_FEE_RATE_BPS.to_string(),
            signature_type: self.config.signature_type,
        })
    }

    /// Update account state from balance API.
    async fn update_account_state(&self) -> anyhow::Result<AccountBalance> {
        // HMAC is computed on the base path only (no query params), matching py-clob-client
        let hmac_path = "/balance-allowance";
        let url = format!(
            "{}/balance-allowance?asset_type=COLLATERAL&signature_type={}",
            self.config.rest_url, self.config.signature_type
        );

        let headers = build_l2_headers(
            &self.wallet_address,
            &self.config.api_key,
            &self.config.api_secret,
            &self.config.api_passphrase,
            "GET",
            hmac_path,
            "",
        )?;

        let response = self
            .http_client
            .get(&url)
            .header(L2Headers::POLY_ADDRESS, &headers.address)
            .header(L2Headers::POLY_API_KEY, &headers.api_key)
            .header(L2Headers::POLY_SIGNATURE, &headers.signature)
            .header(L2Headers::POLY_TIMESTAMP, &headers.timestamp)
            .header(L2Headers::POLY_NONCE, &headers.nonce)
            .header(L2Headers::POLY_PASSPHRASE, &headers.passphrase)
            .send()
            .await?;

        let status = response.status();
        let response_text = response.text().await?;

        if !status.is_success() {
            return Err(anyhow::anyhow!(
                "Balance query failed with status {}: {}",
                status,
                response_text
            ));
        }

        // The Polymarket API response structure:
        // { "allowances": { "0x...": "123456" }, "balance": "123456" }
        // We use "balance" field which represents the actual USDC balance
        log::debug!("Polymarket balance API response: {}", response_text);
        let balance_json: serde_json::Value = serde_json::from_str(&response_text)?;
        
        // Try "balance" first, then fall back to "allowance" for legacy API
        let balance_str = balance_json
            .get("balance")
            .and_then(|b| b.as_str())
            .or_else(|| balance_json.get("allowance").and_then(|b| b.as_str()))
            .unwrap_or_else(|| {
                log::warn!("No 'balance' or 'allowance' field in response, raw: {}", balance_json);
                "0"
            });
        let balance_units: u64 = balance_str.parse().unwrap_or(0);
        log::info!("Polymarket balance parsed: {} units = {} USDC", balance_units, balance_units as f64 / 1_000_000.0);
        let balance_usdc = balance_units as f64 / 1_000_000.0; // 6 decimals

        let usdc = Currency::from("USDC");
        let total = Money::new(balance_usdc, usdc);

        Ok(AccountBalance::new(total, Money::new(0.0, usdc), total))
    }

    /// Truncate deduplication caches to limit memory usage.
    fn truncate_caches(&mut self) {
        while self.processed_fills.len() > PROCESSED_LIMIT {
            self.processed_fills.shift_remove_index(0);
        }
        while self.processed_trades.len() > PROCESSED_LIMIT {
            self.processed_trades.shift_remove_index(0);
        }
        while self.finalized_trades.len() > PROCESSED_LIMIT {
            self.finalized_trades.shift_remove_index(0);
        }
    }

    /// Record a processed fill for deduplication.
    pub fn record_processed_fill(&mut self, trade_id: TradeId, venue_order_id: VenueOrderId) {
        self.processed_fills.insert((trade_id, venue_order_id), ());
        self.truncate_caches();
    }

    /// Record a processed trade for deduplication.
    pub fn record_processed_trade(&mut self, trade_id: TradeId, status: PolymarketTradeStatus) {
        // Move finalized trades to separate cache
        if matches!(
            status,
            PolymarketTradeStatus::Confirmed | PolymarketTradeStatus::Mined
        ) {
            self.finalized_trades.insert(trade_id, ());
            self.processed_trades.shift_remove(&trade_id);
        } else {
            self.processed_trades.insert(trade_id, status);
        }
        self.truncate_caches();
    }

    /// Check if a fill has already been processed.
    pub fn is_fill_processed(&self, trade_id: &TradeId, venue_order_id: &VenueOrderId) -> bool {
        self.processed_fills
            .contains_key(&(*trade_id, *venue_order_id))
    }

    /// Check if a trade has been finalized.
    pub fn is_trade_finalized(&self, trade_id: &TradeId) -> bool {
        self.finalized_trades.contains_key(trade_id)
    }

    /// Get the previous status of a trade.
    pub fn get_trade_status(&self, trade_id: &TradeId) -> Option<PolymarketTradeStatus> {
        self.processed_trades.get(trade_id).copied()
    }

    /// Signal that an order has been acknowledged.
    pub fn signal_order_ack(&self, venue_order_id: &VenueOrderId) {
        if let Some(notifier) = self.ack_notifiers_order.get(venue_order_id) {
            notifier.notify_one();
        }
    }

    /// Signal that a trade has been acknowledged.
    pub fn signal_trade_ack(&self, venue_order_id: &VenueOrderId) {
        if let Some(notifier) = self.ack_notifiers_trade.get(venue_order_id) {
            notifier.notify_one();
        }
    }

    /// Get or create an order ack notifier.
    pub fn get_order_ack_notifier(&mut self, venue_order_id: VenueOrderId) -> Arc<Notify> {
        self.ack_notifiers_order
            .entry(venue_order_id)
            .or_insert_with(|| Arc::new(Notify::new()))
            .clone()
    }

    /// Get or create a trade ack notifier.
    pub fn get_trade_ack_notifier(&mut self, venue_order_id: VenueOrderId) -> Arc<Notify> {
        self.ack_notifiers_trade
            .entry(venue_order_id)
            .or_insert_with(|| Arc::new(Notify::new()))
            .clone()
    }

    /// Remove order ack notifier after use.
    pub fn remove_order_ack_notifier(&mut self, venue_order_id: &VenueOrderId) {
        self.ack_notifiers_order.remove(venue_order_id);
    }

    /// Remove trade ack notifier after use.
    pub fn remove_trade_ack_notifier(&mut self, venue_order_id: &VenueOrderId) {
        self.ack_notifiers_trade.remove(venue_order_id);
    }

    /// Extract token ID from instrument ID.
    ///
    /// Instrument ID format: "{token_id}.POLYMARKET"
    fn extract_token_id(
        instrument_id: &nautilus_model::identifiers::InstrumentId,
        token_map: &Option<TokenMap>,
    ) -> anyhow::Result<String> {
        let symbol = instrument_id.symbol.as_str();

        // Look up the CTF token ID from the shared token map
        if let Some(map) = token_map {
            if let Ok(guard) = map.read() {
                if let Some(token_id) = guard.get(symbol) {
                    log::info!(
                        "extract_token_id: symbol={} → token_id={}...{}",
                        symbol,
                        &token_id[..8.min(token_id.len())],
                        &token_id[token_id.len().saturating_sub(6)..]
                    );
                    return Ok(token_id.clone());
                }
            }
        }

        // Fallback: if the symbol looks numeric (actual token ID), use it directly
        if symbol.chars().all(|c| c.is_ascii_digit()) {
            return Ok(symbol.to_string());
        }

        Err(anyhow::anyhow!(
            "No token ID resolved for instrument {instrument_id}. \
             Ensure the token resolver is running and has mapped this instrument."
        ))
    }
}

#[async_trait(?Send)]
impl ExecutionClient for PolymarketExecutionClient {
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
        // TODO: Return cached account state
        None
    }

    fn generate_account_state(
        &self,
        _balances: Vec<AccountBalance>,
        _margins: Vec<MarginBalance>,
        _reported: bool,
        _ts_event: UnixNanos,
    ) -> anyhow::Result<()> {
        // TODO: Generate and publish AccountState event
        Ok(())
    }

    fn start(&mut self) -> anyhow::Result<()> {
        log::info!("PolymarketExecutionClient: starting...");
        self.is_connected = true;
        Ok(())
    }

    fn stop(&mut self) -> anyhow::Result<()> {
        log::info!("PolymarketExecutionClient: stopping...");
        self.is_connected = false;
        Ok(())
    }

    async fn connect(&mut self) -> anyhow::Result<()> {
        log::info!("PolymarketExecutionClient: connecting...");

        // Initialize WebSocket connection for real-time updates
        let ws_url = self.config.ws_url.replace("/ws/market", "/ws/user");
        self.ws = Some(PolymarketWebSocket::new(
            ws_url,
            self.config.api_key.clone(),
            self.config.api_secret.clone(),
            self.config.api_passphrase.clone(),
        ));

        if let Some(ref mut ws) = self.ws {
            let _rx = ws.connect().await?;
            log::info!("PolymarketExecutionClient: WebSocket connected");
        }

        // Load initial account state
        match self.update_account_state().await {
            Ok(balance) => {
                log::info!(
                    "PolymarketExecutionClient: account balance loaded: {:?}",
                    balance
                );
            }
            Err(e) => {
                log::warn!(
                    "PolymarketExecutionClient: failed to load account balance: {}",
                    e
                );
            }
        }

        self.is_connected = true;
        log::info!("PolymarketExecutionClient: connected");
        Ok(())
    }

    async fn disconnect(&mut self) -> anyhow::Result<()> {
        log::info!("PolymarketExecutionClient: disconnecting...");

        if let Some(ref mut ws) = self.ws {
            ws.disconnect().await;
        }
        self.ws = None;

        self.is_connected = false;
        Ok(())
    }

    fn submit_order(&self, cmd: &SubmitOrder) -> anyhow::Result<()> {
        // Get order from cache using client_order_id
        let order = self.get_order(&cmd.client_order_id)?;

        log::info!(
            "PolymarketExecutionClient: submit_order {:?}",
            order.client_order_id()
        );

        // Validate order type
        match order.order_type() {
            OrderType::Limit | OrderType::Market => {}
            other => {
                return Err(anyhow::anyhow!(
                    "Order type {:?} not supported on Polymarket; use MARKET or LIMIT",
                    other
                ));
            }
        }

        // Validate time-in-force
        if Self::convert_tif(order.time_in_force()).is_none() {
            return Err(anyhow::anyhow!(
                "Time-in-force {:?} not supported on Polymarket; use FOK, GTC, GTD, or IOC",
                order.time_in_force()
            ));
        }

        // Extract token ID from instrument
        let token_id = Self::extract_token_id(&order.instrument_id(), &self.token_map)?;

        // Create signed order
        let order_type = Self::convert_tif(order.time_in_force()).ok_or_else(|| {
            anyhow::anyhow!("Unsupported time-in-force: {:?}", order.time_in_force())
        })?;
        let signed_order = self.create_signed_order(&order, &token_id)?;

        // Clone values needed for async block
        let http_client = self.http_client.clone();
        let rest_url = self.config.rest_url.clone();
        let api_key = self.config.api_key.clone();
        let api_secret = self.config.api_secret.clone();
        let order_results = self.order_results.clone();
        let api_passphrase = self.config.api_passphrase.clone();
        let wallet_address = self.wallet_address.clone();
        let client_order_id = order.client_order_id();

        // Spawn async task to submit order
        // Note: In a real implementation, this would use the event loop properly
        // and generate OrderSubmitted, OrderAccepted/Rejected events
        tokio::spawn(async move {
            let request = CreateOrderRequest {
                order: signed_order.clone(),
                owner: api_key.clone(),
                order_type,
            };

            let body = match serde_json::to_string(&request) {
                Ok(b) => b,
                Err(e) => {
                    log::error!("Failed to serialize order request: {}", e);
                    return;
                }
            };

            log::info!("Order payload: {}", body);

            let path = "/order";
            let url = format!("{}{}", rest_url, path);

            let headers = match build_l2_headers(
                &wallet_address,
                &api_key,
                &api_secret,
                &api_passphrase,
                "POST",
                path,
                &body,
            ) {
                Ok(h) => h,
                Err(e) => {
                    log::error!("Failed to build headers: {}", e);
                    return;
                }
            };

            let response = http_client
                .post(&url)
                .header(L2Headers::POLY_ADDRESS, &headers.address)
                .header(L2Headers::POLY_API_KEY, &headers.api_key)
                .header(L2Headers::POLY_SIGNATURE, &headers.signature)
                .header(L2Headers::POLY_TIMESTAMP, &headers.timestamp)
                .header(L2Headers::POLY_NONCE, &headers.nonce)
                .header(L2Headers::POLY_PASSPHRASE, &headers.passphrase)
                .header("Content-Type", "application/json")
                .body(body)
                .send()
                .await;

            // Helper: write result to shared order_results map
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
                        match serde_json::from_str::<CreateOrderResponse>(&text) {
                            Ok(order_resp) => {
                                if order_resp.success {
                                    log::info!(
                                        "Order {:?} submitted successfully: venue_order_id={:?} status={:?}",
                                        client_order_id,
                                        order_resp.order_id,
                                        order_resp.status
                                    );
                                    write_result(true);
                                } else {
                                    log::error!(
                                        "Order {:?} rejected: {:?}",
                                        client_order_id,
                                        order_resp.error_msg
                                    );
                                    write_result(false);
                                }
                            }
                            Err(e) => {
                                log::error!(
                                    "Failed to parse order response: {} - body: {}",
                                    e,
                                    text
                                );
                                write_result(false);
                            }
                        }
                    } else {
                        log::error!(
                            "Order {:?} submission failed with status {}: {}",
                            client_order_id,
                            status,
                            text
                        );
                        write_result(false);
                    }
                }
                Err(e) => {
                    log::error!("Order {:?} submission HTTP error: {}", client_order_id, e);
                    write_result(false);
                }
            }
        });

        Ok(())
    }

    fn cancel_order(&self, cmd: &CancelOrder) -> anyhow::Result<()> {
        log::info!(
            "PolymarketExecutionClient: cancel_order {:?}",
            cmd.client_order_id
        );

        // Look up venue order ID
        let venue_order_id = match cmd.venue_order_id {
            Some(ref id) => *id,
            None => {
                // Try to find in order_id_map
                match self.order_id_map.get(&cmd.client_order_id) {
                    Some(id) => *id,
                    None => {
                        return Err(anyhow::anyhow!(
                            "Cannot cancel order {:?}: venue_order_id not found",
                            cmd.client_order_id
                        ));
                    }
                }
            }
        };

        // Clone values for async block
        let http_client = self.http_client.clone();
        let rest_url = self.config.rest_url.clone();
        let wallet_address = self.wallet_address.clone();
        let api_key = self.config.api_key.clone();
        let api_secret = self.config.api_secret.clone();
        let api_passphrase = self.config.api_passphrase.clone();
        let client_order_id = cmd.client_order_id;

        // Spawn async task to cancel order
        tokio::spawn(async move {
            let path = format!("/order/{}", venue_order_id.as_str());
            let url = format!("{}{}", rest_url, path);

            let headers = match build_l2_headers(
                &wallet_address,
                &api_key,
                &api_secret,
                &api_passphrase,
                "DELETE",
                &path,
                "",
            ) {
                Ok(h) => h,
                Err(e) => {
                    log::error!("Failed to build headers for cancel: {}", e);
                    return;
                }
            };

            let response = http_client
                .delete(&url)
                .header(L2Headers::POLY_ADDRESS, &headers.address)
                .header(L2Headers::POLY_API_KEY, &headers.api_key)
                .header(L2Headers::POLY_SIGNATURE, &headers.signature)
                .header(L2Headers::POLY_TIMESTAMP, &headers.timestamp)
                .header(L2Headers::POLY_NONCE, &headers.nonce)
                .header(L2Headers::POLY_PASSPHRASE, &headers.passphrase)
                .send()
                .await;

            match response {
                Ok(resp) => {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();

                    if status.is_success() {
                        match serde_json::from_str::<CancelOrderResponse>(&text) {
                            Ok(cancel_resp) => {
                                if cancel_resp.success {
                                    log::info!("Order {:?} canceled successfully", client_order_id);
                                    // TODO: Generate OrderCanceled event
                                } else {
                                    log::error!(
                                        "Order {:?} cancel rejected: {:?}",
                                        client_order_id,
                                        cancel_resp.error_msg
                                    );
                                    // TODO: Generate OrderCancelRejected event
                                }
                            }
                            Err(e) => {
                                log::error!(
                                    "Failed to parse cancel response: {} - body: {}",
                                    e,
                                    text
                                );
                            }
                        }
                    } else {
                        log::error!(
                            "Order {:?} cancel failed with status {}: {}",
                            client_order_id,
                            status,
                            text
                        );
                        // TODO: Generate OrderCancelRejected event
                    }
                }
                Err(e) => {
                    log::error!("Order {:?} cancel HTTP error: {}", client_order_id, e);
                    // TODO: Generate OrderCancelRejected event
                }
            }
        });

        Ok(())
    }

    fn modify_order(&self, cmd: &ModifyOrder) -> anyhow::Result<()> {
        log::info!(
            "PolymarketExecutionClient: modify_order {:?} - not supported",
            cmd.client_order_id
        );
        // Polymarket CLOB does not support order modification
        anyhow::bail!("Order modification not supported by Polymarket CLOB. Cancel and resubmit.")
    }
}
