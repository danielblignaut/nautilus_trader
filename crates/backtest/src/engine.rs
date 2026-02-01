// -------------------------------------------------------------------------------------------------
//  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
//  https://nautechsystems.io
//
//  Licensed under the GNU Lesser General Public License Version 3.0 (the "License");
//  You may not use this file except in compliance with the License.
//  You may obtain a copy of the License at https://www.gnu.org/licenses/lgpl-3.0.en.html
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.
// -------------------------------------------------------------------------------------------------

// Under development
#![allow(dead_code)]
#![allow(unused_variables)]

//! The core `BacktestEngine` for backtesting on historical data.

use std::{
    any::Any,
    cell::RefCell,
    collections::HashSet,
    fmt::Debug,
    panic::{catch_unwind, AssertUnwindSafe},
    rc::Rc,
};

use ahash::AHashMap;
use nautilus_common::{
    actor::DataActor,
    clock::TestClock,
    component::Component,
    messages::execution::TradingCommand,
    msgbus::{self, MessagingSwitchboard},
    runner::{
        set_data_cmd_sender, set_exec_cmd_sender, set_time_event_sender, SyncDataCommandSender,
        TimeEventSender, TradingCommandSender,
    },
    timer::TimeEventHandler,
};
use nautilus_core::{UnixNanos, UUID4};
use nautilus_data::client::DataClientAdapter;
use nautilus_execution::models::{fee::FeeModelAny, fill::FillModel, latency::LatencyModel};
use nautilus_model::{
    data::{Data, HasTsInit},
    enums::{AccountType, BookType, OmsType},
    identifiers::{AccountId, ClientId, InstrumentId, Venue},
    instruments::{Instrument, InstrumentAny},
    types::{Currency, Money},
};
use nautilus_system::{config::NautilusKernelConfig, kernel::NautilusKernel};
use nautilus_trading::Strategy;
use rust_decimal::Decimal;

use crate::{
    accumulator::TimeEventAccumulator, config::BacktestEngineConfig,
    data_client::BacktestDataClient, exchange::SimulatedExchange,
    execution_client::BacktestExecutionClient, modules::SimulationModule,
};

/// Synchronous time event sender for backtest (no-op, events handled by accumulator).
#[derive(Debug)]
struct SyncTimeEventSender;

impl TimeEventSender for SyncTimeEventSender {
    fn send(&self, _handler: TimeEventHandler) {
        // In backtest, time events are handled by the TimeEventAccumulator directly
    }
}

/// Synchronous trading command sender for backtest.
#[derive(Debug)]
struct SyncTradingCommandSender;

impl TradingCommandSender for SyncTradingCommandSender {
    fn execute(&self, command: TradingCommand) {
        let endpoint = MessagingSwitchboard::exec_engine_execute();
        msgbus::send_trading_command(endpoint, command);
    }
}

/// Core backtesting engine for running event-driven strategy backtests on historical data.
///
/// The `BacktestEngine` provides a high-fidelity simulation environment that processes
/// historical market data chronologically through an event-driven architecture. It maintains
/// simulated exchanges with realistic order matching and execution, allowing strategies
/// to be tested exactly as they would run in live trading:
///
/// - Event-driven data replay with configurable latency models.
/// - Multi-venue and multi-asset support.
/// - Realistic order matching and execution simulation.
/// - Strategy and portfolio performance analysis.
/// - Seamless transition from backtesting to live trading.
pub struct BacktestEngine {
    instance_id: UUID4,
    config: BacktestEngineConfig,
    pub kernel: NautilusKernel,
    accumulator: TimeEventAccumulator,
    run_config_id: Option<UUID4>,
    run_id: Option<UUID4>,
    venues: AHashMap<Venue, Rc<RefCell<SimulatedExchange>>>,
    has_data: HashSet<InstrumentId>,
    has_book_data: HashSet<InstrumentId>,
    data: Vec<Data>,
    index: usize,
    iteration: usize,
    run_started: Option<UnixNanos>,
    run_finished: Option<UnixNanos>,
    backtest_start: Option<UnixNanos>,
    backtest_end: Option<UnixNanos>,
}

impl Debug for BacktestEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(stringify!(BacktestEngine))
            .field("instance_id", &self.instance_id)
            .field("run_config_id", &self.run_config_id)
            .field("run_id", &self.run_id)
            .finish()
    }
}

impl BacktestEngine {
    /// Create a new [`BacktestEngine`] instance.
    ///
    /// # Errors
    ///
    /// Returns an error if the core `NautilusKernel` fails to initialize.
    pub fn new(config: BacktestEngineConfig) -> anyhow::Result<Self> {
        let kernel = NautilusKernel::new("BacktestEngine".to_string(), config.clone())?;

        Ok(Self {
            instance_id: kernel.instance_id,
            config,
            accumulator: TimeEventAccumulator::new(),
            kernel,
            run_config_id: None,
            run_id: None,
            venues: AHashMap::new(),
            has_data: HashSet::new(),
            has_book_data: HashSet::new(),
            data: Vec::new(),
            index: 0,
            iteration: 0,
            run_started: None,
            run_finished: None,
            backtest_start: None,
            backtest_end: None,
        })
    }

    /// # Errors
    ///
    /// Returns an error if initializing the simulated exchange for the venue fails.
    #[allow(clippy::too_many_arguments)]
    pub fn add_venue(
        &mut self,
        venue: Venue,
        oms_type: OmsType,
        account_type: AccountType,
        book_type: BookType,
        starting_balances: Vec<Money>,
        base_currency: Option<Currency>,
        default_leverage: Option<Decimal>,
        leverages: AHashMap<InstrumentId, Decimal>,
        modules: Vec<Box<dyn SimulationModule>>,
        fill_model: FillModel,
        fee_model: FeeModelAny,
        latency_model: Option<Box<dyn LatencyModel>>,
        routing: Option<bool>,
        reject_stop_orders: Option<bool>,
        support_gtd_orders: Option<bool>,
        support_contingent_orders: Option<bool>,
        use_position_ids: Option<bool>,
        use_random_ids: Option<bool>,
        use_reduce_only: Option<bool>,
        use_message_queue: Option<bool>,
        use_market_order_acks: Option<bool>,
        bar_execution: Option<bool>,
        bar_adaptive_high_low_ordering: Option<bool>,
        trade_execution: Option<bool>,
        liquidity_consumption: Option<bool>,
        allow_cash_borrowing: Option<bool>,
        frozen_account: Option<bool>,
        price_protection_points: Option<u32>,
    ) -> anyhow::Result<()> {
        let default_leverage: Decimal = default_leverage.unwrap_or_else(|| {
            if account_type == AccountType::Margin {
                Decimal::from(10)
            } else {
                Decimal::from(1)
            }
        });

        let exchange = SimulatedExchange::new(
            venue,
            oms_type,
            account_type,
            starting_balances,
            base_currency,
            default_leverage,
            leverages,
            modules,
            self.kernel.cache.clone(),
            self.kernel.clock.clone(),
            fill_model,
            fee_model,
            book_type,
            latency_model,
            bar_execution,
            trade_execution,
            liquidity_consumption, // liquidity_consumption - None defaults to false in SimulatedExchange
            reject_stop_orders,
            support_gtd_orders,
            support_contingent_orders,
            use_position_ids,
            use_random_ids,
            use_reduce_only,
            use_message_queue,
            use_market_order_acks,
            allow_cash_borrowing,
            frozen_account,
            price_protection_points,
        )?;
        let exchange = Rc::new(RefCell::new(exchange));
        self.venues.insert(venue, exchange.clone());

        let account_id = AccountId::from(format!("{venue}-001").as_str());

        let exec_client = BacktestExecutionClient::new(
            self.config.trader_id(),
            account_id,
            exchange.clone(),
            self.kernel.cache.clone(),
            self.kernel.clock.clone(),
            routing,
            frozen_account,
        );

        exchange
            .borrow_mut()
            .register_client(Rc::new(exec_client.clone()));

        self.kernel
            .exec_engine
            .borrow_mut()
            .register_client(Box::new(exec_client))?;

        // Register data client with the data engine
        let data_client = BacktestDataClient::new(
            ClientId::from(venue.as_str()),
            venue,
            self.kernel.cache.clone(),
        );
        let data_adapter = DataClientAdapter::new(
            ClientId::from(venue.as_str()),
            Some(venue),
            false,
            true,
            Box::new(data_client),
        );
        self.kernel
            .data_engine
            .borrow_mut()
            .register_client(data_adapter, Some(venue));

        log::info!("Adding exchange {venue} to engine");

        Ok(())
    }

    pub fn change_fill_model(&mut self, venue: Venue, fill_model: FillModel) {
        if let Some(exchange) = self.venues.get_mut(&venue) {
            exchange.borrow_mut().set_fill_model(fill_model);
        } else {
            log::warn!(
                "BacktestEngine::change_fill_model called for unknown venue {venue}. Ignoring."
            );
        }
    }

    /// Adds an instrument to the backtest engine for the specified venue.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The instrument's associated venue has not been added via `add_venue`.
    /// - Attempting to add a `CurrencyPair` instrument for a single-currency CASH account.
    ///
    /// # Panics
    ///
    /// Panics if adding the instrument to the simulated exchange fails.
    pub fn add_instrument(&mut self, instrument: InstrumentAny) -> anyhow::Result<()> {
        let instrument_id = instrument.id();
        if let Some(exchange) = self.venues.get_mut(&instrument.id().venue) {
            // check if instrument is of variant CurrencyPair
            if matches!(instrument, InstrumentAny::CurrencyPair(_))
                && exchange.borrow().account_type != AccountType::Margin
                && exchange.borrow().base_currency.is_some()
            {
                anyhow::bail!(
                    "Cannot add a `CurrencyPair` instrument {instrument_id} for a venue with a single-currency CASH account"
                )
            }
            exchange
                .borrow_mut()
                .add_instrument(instrument.clone())
                .unwrap();
        } else {
            anyhow::bail!(
                "Cannot add an `Instrument` object without first adding its associated venue {}",
                instrument.id().venue
            )
        }

        // Check client has been registered
        self.add_market_data_client_if_not_exists(instrument.id().venue);

        self.kernel
            .data_engine
            .borrow_mut()
            .process(&instrument as &dyn Any);
        log::info!(
            "Added instrument {} to exchange {}",
            instrument_id,
            instrument_id.venue
        );
        Ok(())
    }

    pub fn add_data(
        &mut self,
        data: Vec<Data>,
        client_id: Option<ClientId>,
        validate: bool,
        sort: bool,
    ) {
        if data.is_empty() {
            log::warn!("add_data called with empty data slice – ignoring");
            return;
        }

        // If requested, sort by ts_init so internal stream is monotonic.
        let mut to_add = data;
        if sort {
            to_add.sort_by_key(HasTsInit::ts_init);
        }

        // Instrument & book tracking using Data helpers
        if validate {
            for item in &to_add {
                let instr_id = item.instrument_id();
                self.has_data.insert(instr_id);

                if item.is_order_book_data() {
                    self.has_book_data.insert(instr_id);
                }

                // Ensure appropriate market data client exists
                self.add_market_data_client_if_not_exists(instr_id.venue);
            }
        }

        // Extend master data vector and ensure internal iterator (index) remains valid.
        self.data.extend(to_add);

        if sort {
            self.data.sort_by_key(HasTsInit::ts_init);
        }

        log::info!(
            "Added {} data element{} to BacktestEngine",
            self.data.len(),
            if self.data.len() == 1 { "" } else { "s" }
        );
    }

    pub fn add_actor<T>(&mut self, actor: T) -> anyhow::Result<()>
    where
        T: DataActor + Component + Debug + 'static,
    {
        self.kernel.trader.add_actor(actor)
    }

    pub fn add_actors<T>(&mut self, actors: Vec<T>) -> anyhow::Result<()>
    where
        T: DataActor + Component + Debug + 'static,
    {
        for a in actors {
            self.add_actor(a)?;
        }
        Ok(())
    }

    pub fn add_strategy<T>(&mut self, strategy: T) -> anyhow::Result<()>
    where
        T: Strategy + Component + Debug + 'static,
    {
        self.kernel.trader.add_strategy(strategy)
    }

    pub fn add_strategies<T>(&mut self, strategies: Vec<T>) -> anyhow::Result<()>
    where
        T: Strategy + Component + Debug + 'static,
    {
        for s in strategies {
            self.add_strategy(s)?;
        }
        Ok(())
    }

    pub fn add_exec_algorithm<T>(&mut self, exec_algorithm: T) -> anyhow::Result<()>
    where
        T: DataActor + Component + Debug + 'static,
    {
        self.kernel.trader.add_exec_algorithm(exec_algorithm)
    }

    pub fn add_exec_algorithms<T>(&mut self, exec_algorithms: Vec<T>) -> anyhow::Result<()>
    where
        T: DataActor + Component + Debug + 'static,
    {
        for ea in exec_algorithms {
            self.add_exec_algorithm(ea)?;
        }
        Ok(())
    }

    pub fn reset(&mut self) {
        for exchange in self.venues.values() {
            exchange.borrow_mut().reset();
        }
        self.kernel.reset();
        self.data.clear();
        self.has_data.clear();
        self.has_book_data.clear();
        self.index = 0;
        self.iteration = 0;
        self.run_started = None;
        self.run_finished = None;
        self.backtest_start = None;
        self.backtest_end = None;
        self.run_config_id = None;
        self.run_id = None;
        log::info!("BacktestEngine reset");
    }

    pub fn clear_data(&mut self) {
        self.data.clear();
        self.has_data.clear();
        self.has_book_data.clear();
        self.index = 0;
        log::info!("BacktestEngine data cleared");
    }

    pub fn clear_strategies(&mut self) {
        log::warn!("clear_strategies not yet supported by Trader API");
    }

    pub fn clear_exec_algorithms(&mut self) {
        log::warn!("clear_exec_algorithms not yet supported by Trader API");
    }

    pub fn dispose(&mut self) {
        self.kernel.dispose();
        self.venues.clear();
        log::info!("BacktestEngine disposed");
    }

    /// Run the backtest engine over loaded data.
    ///
    /// Data is preserved across calls, allowing multiple `run()` invocations with
    /// different time windows. Use `self.index` to seek into the data for the next
    /// run (matching the Python engine's non-destructive iterator approach).
    ///
    /// # Errors
    ///
    /// Returns an error if no data has been loaded or if engine initialization fails.
    pub fn run(&mut self, start: Option<UnixNanos>, end: Option<UnixNanos>) -> anyhow::Result<()> {
        anyhow::ensure!(!self.data.is_empty(), "No data to run backtest");

        let start_ns = start.unwrap_or_else(|| self.data.first().unwrap().ts_init());
        let end_ns = end.unwrap_or_else(|| self.data.last().unwrap().ts_init());

        // First run initialization
        if self.iteration == 0 {
            self.run_id = Some(UUID4::new());
            self.run_config_id = Some(UUID4::new());

            // Initialize sync command senders for backtest
            set_data_cmd_sender(std::sync::Arc::new(SyncDataCommandSender));
            set_time_event_sender(std::sync::Arc::new(SyncTimeEventSender));
            set_exec_cmd_sender(std::sync::Arc::new(SyncTradingCommandSender));

            for exchange in self.venues.values() {
                exchange.borrow_mut().initialize_account();
            }

            self.kernel.data_engine.borrow_mut().start();
            self.kernel.exec_engine.borrow_mut().start();
            self.kernel.risk_engine.borrow_mut().start();
            self.kernel.trader.initialize()?;
            self.kernel.trader.start()?;
        }

        self.run_started = Some(self.kernel.clock.borrow().timestamp_ns());
        self.backtest_start = Some(start_ns);
        self.backtest_end = Some(end_ns);
        self.log_pre_run();

        // Seek index to first element >= start_ns
        while self.index < self.data.len() && self.data[self.index].ts_init() < start_ns {
            self.index += 1;
        }

        let mut last_ns = UnixNanos::default();
        while self.index < self.data.len() {
            let data = &self.data[self.index];
            let ts = data.ts_init();
            if ts > end_ns {
                break;
            }

            // Advance time when timestamp changes
            if ts > last_ns {
                // advance_clock with set_time=true internally calls set_time,
                // so no separate set_time call is needed.
                let mut clock_guard = self.kernel.clock.borrow_mut();
                let test_clock = clock_guard
                    .as_any_mut()
                    .downcast_mut::<TestClock>()
                    .expect("BacktestEngine requires a TestClock");
                self.accumulator.advance_clock(test_clock, ts, true);
                drop(clock_guard);
                last_ns = ts;
            }

            // Route data to simulated exchange
            let venue = data.instrument_id().venue;
            if let Some(exchange) = self.venues.get(&venue) {
                let mut ex = exchange.borrow_mut();
                match data {
                    Data::Delta(d) => ex.process_order_book_delta(*d),
                    Data::Deltas(d) => ex.process_order_book_deltas((**d).clone()),
                    Data::Quote(q) => ex.process_quote_tick(q),
                    Data::Trade(t) => ex.process_trade_tick(t),
                    Data::Bar(b) => ex.process_bar(*b),
                    _ => {}
                }
            }

            // Clone data for the data engine (process_data takes ownership)
            let data_owned = self.data[self.index].clone();
            self.kernel
                .data_engine
                .borrow_mut()
                .process_data(data_owned);

            // Drain deferred order events before exchange processing
            // (OrderSubmitted must be applied before exchange can fill orders)
            crate::execution_client::drain_deferred_order_events();

            // Process exchange queues (catch RefCell borrow panics and propagate as errors)
            for exchange in self.venues.values() {
                let exchange_ref = Rc::clone(exchange);
                let result = catch_unwind(AssertUnwindSafe(|| {
                    exchange_ref.borrow_mut().process(ts);
                }));
                if let Err(panic_payload) = result {
                    let msg = if let Some(s) = panic_payload.downcast_ref::<&str>() {
                        s.to_string()
                    } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                        s.clone()
                    } else {
                        "unknown panic".to_string()
                    };
                    log::error!("FATAL: RefCell borrowing panic in exchange processing: {msg}");
                    return Err(anyhow::anyhow!(
                        "Backtest terminated due to RefCell borrowing panic: {msg}"
                    ));
                }
            }

            // Drain any new deferred events from exchange processing
            crate::execution_client::drain_deferred_order_events();

            // Process accumulated time events
            let handlers = self.accumulator.drain();
            if !handlers.is_empty() {
                self.process_raw_time_event_handlers(handlers, ts, false, true);
            }

            self.index += 1;
            self.iteration += 1;
        }

        self.run_finished = Some(self.kernel.clock.borrow().timestamp_ns());
        self.log_post_run();
        self.end()?;
        Ok(())
    }

    /// End the backtest, stopping the trader and engines.
    ///
    /// # Errors
    ///
    /// Returns an error if stopping the trader fails.
    pub fn end(&mut self) -> anyhow::Result<()> {
        if !self.kernel.trader.is_running() {
            return Ok(());
        }
        self.kernel.stop_trader();
        self.kernel.data_engine.borrow_mut().stop();
        self.kernel.exec_engine.borrow_mut().stop();
        self.kernel.risk_engine.borrow_mut().stop();
        self.run_finished = Some(self.kernel.clock.borrow().timestamp_ns());
        log::info!("Backtest ended");
        Ok(())
    }

    pub fn get_result(&self) {
        // TODO: implement full BacktestResult aggregation once portfolio analysis
        // components are available in Rust. For now we simply log and return.
        log::info!("BacktestEngine::get_result called – not yet implemented");
    }

    pub fn process_raw_time_event_handlers(
        &mut self,
        handlers: Vec<TimeEventHandler>,
        ts_now: UnixNanos,
        only_now: bool,
        as_of_now: bool,
    ) {
        let mut last_ts_init: Option<UnixNanos> = None;

        for handler in handlers {
            let ts_event_init = handler.event.ts_event; // event time

            if Self::should_skip_time_event(ts_event_init, ts_now, only_now, as_of_now) {
                continue;
            }

            if last_ts_init != Some(ts_event_init) {
                // First handler for this timestamp – process exchange queues beforehand.
                for exchange in self.venues.values() {
                    exchange.borrow_mut().process(ts_event_init);
                }
                last_ts_init = Some(ts_event_init);
            }

            handler.run();
        }
    }

    pub fn log_pre_run(&self) {
        log::info!("=== BACKTEST PRE-RUN ===");
        log::info!("Run ID: {:?}", self.run_id);
        log::info!("Venues: {}", self.venues.len());
        log::info!("Data events: {}", self.data.len());
        log::info!("Instruments with data: {}", self.has_data.len());
    }

    pub fn log_run(&self) {
        log::info!("=== BACKTEST RUN ===");
        log::info!("Iteration: {}", self.iteration);
    }

    pub fn log_post_run(&self) {
        log::info!("=== BACKTEST POST-RUN ===");
        log::info!("Total iterations: {}", self.iteration);
        if let (Some(started), Some(finished)) = (self.run_started, self.run_finished) {
            let elapsed_ns = finished.as_u64() - started.as_u64();
            let elapsed_s = elapsed_ns as f64 / 1e9;
            let rate = if elapsed_s > 0.0 {
                self.iteration as f64 / elapsed_s
            } else {
                0.0
            };
            log::info!("Elapsed: {:.2}s ({:.0} events/s)", elapsed_s, rate);
        }
    }

    pub fn add_data_client_if_not_exists(&mut self, client_id: ClientId) {
        if self
            .kernel
            .data_engine
            .borrow()
            .registered_clients()
            .contains(&client_id)
        {
            return;
        }

        // Create a generic, venue-agnostic backtest data client. We use a dummy
        // venue derived from the client id for uniqueness.
        let venue = Venue::from(client_id.as_str());
        let backtest_client = BacktestDataClient::new(client_id, venue, self.kernel.cache.clone());
        let data_client_adapter = DataClientAdapter::new(
            backtest_client.client_id,
            None, // no specific venue association
            false,
            false,
            Box::new(backtest_client),
        );

        self.kernel
            .data_engine
            .borrow_mut()
            .register_client(data_client_adapter, None);
    }

    // Helper matching Cython semantics for determining whether to skip
    // processing a time event.
    fn should_skip_time_event(
        ts_event_init: UnixNanos,
        ts_now: UnixNanos,
        only_now: bool,
        as_of_now: bool,
    ) -> bool {
        if only_now {
            ts_event_init != ts_now
        } else if as_of_now {
            ts_event_init > ts_now
        } else {
            ts_event_init >= ts_now
        }
    }

    // TODO: We might want venue to be optional for multi-venue clients
    pub fn add_market_data_client_if_not_exists(&mut self, venue: Venue) {
        let client_id = ClientId::from(venue.as_str());
        if !self
            .kernel
            .data_engine
            .borrow()
            .registered_clients()
            .contains(&client_id)
        {
            let backtest_client =
                BacktestDataClient::new(client_id, venue, self.kernel.cache.clone());
            let data_client_adapter = DataClientAdapter::new(
                client_id,
                Some(venue), // TBD
                false,
                false,
                Box::new(backtest_client),
            );
            self.kernel
                .data_engine
                .borrow_mut()
                .register_client(data_client_adapter, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use ahash::AHashMap;
    use nautilus_execution::models::{fee::FeeModelAny, fill::FillModel};
    use nautilus_model::{
        enums::{AccountType, BookType, OmsType},
        identifiers::{ClientId, Venue},
        instruments::{
            stubs::crypto_perpetual_ethusdt, CryptoPerpetual, Instrument, InstrumentAny,
        },
        types::Money,
    };
    use rstest::rstest;

    use crate::{config::BacktestEngineConfig, engine::BacktestEngine};

    #[allow(clippy::missing_panics_doc)]
    fn get_backtest_engine(config: Option<BacktestEngineConfig>) -> BacktestEngine {
        let config = config.unwrap_or_default();
        let mut engine = BacktestEngine::new(config).unwrap();
        engine
            .add_venue(
                Venue::from("BINANCE"),
                OmsType::Netting,
                AccountType::Margin,
                BookType::L2_MBP,
                vec![Money::from("1_000_000 USD")],
                None,
                None,
                AHashMap::new(),
                vec![],
                FillModel::default(),
                FeeModelAny::default(),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        engine
    }

    #[rstest]
    fn test_engine_venue_and_instrument_initialization(crypto_perpetual_ethusdt: CryptoPerpetual) {
        let venue = Venue::from("BINANCE");
        let client_id = ClientId::from(venue.as_str());
        let instrument = InstrumentAny::CryptoPerpetual(crypto_perpetual_ethusdt);
        let instrument_id = instrument.id();
        let mut engine = get_backtest_engine(None);
        engine.add_instrument(instrument).unwrap();

        // Check the venue has been added
        assert_eq!(engine.venues.len(), 1);
        assert!(engine.venues.contains_key(&venue));

        // Check the instrument has been added
        assert!(engine
            .venues
            .get(&venue)
            .is_some_and(|venue| venue.borrow().get_matching_engine(&instrument_id).is_some()));
        assert_eq!(
            engine
                .kernel
                .data_engine
                .borrow()
                .registered_clients()
                .len(),
            1
        );
        assert!(engine
            .kernel
            .data_engine
            .borrow()
            .registered_clients()
            .contains(&client_id));
    }
}
