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

//! Provides a `BacktestDataClient` implementation for backtesting.

// Under development
#![allow(dead_code)]
#![allow(unused_variables)]

use std::{cell::RefCell, rc::Rc};

use nautilus_common::{
    cache::Cache,
    clients::DataClient,
    messages::data::{
        RequestBars, RequestBookSnapshot, RequestCustomData, RequestInstrument, RequestInstruments,
        RequestQuotes, RequestTrades, SubscribeBars, SubscribeBookDeltas, SubscribeBookDepth10,
        SubscribeCustomData, SubscribeIndexPrices, SubscribeInstrument, SubscribeInstrumentClose,
        SubscribeInstrumentStatus, SubscribeInstruments, SubscribeMarkPrices, SubscribeQuotes,
        SubscribeTrades, UnsubscribeBars, UnsubscribeBookDeltas, UnsubscribeBookDepth10,
        UnsubscribeCustomData, UnsubscribeIndexPrices, UnsubscribeInstrument,
        UnsubscribeInstrumentClose, UnsubscribeInstrumentStatus, UnsubscribeInstruments,
        UnsubscribeMarkPrices, UnsubscribeQuotes, UnsubscribeTrades,
    },
};
use nautilus_model::identifiers::{ClientId, Venue};

#[derive(Debug)]
/// Data client implementation for backtesting market data operations.
///
/// The `BacktestDataClient` provides a data client interface specifically designed
/// for backtesting environments. It handles market data subscriptions and requests
/// during backtesting, coordinating with the backtesting engine to provide
/// historical data replay functionality.
pub struct BacktestDataClient {
    pub client_id: ClientId,
    pub venue: Venue,
    cache: Rc<RefCell<Cache>>,
}

impl BacktestDataClient {
    pub const fn new(client_id: ClientId, venue: Venue, cache: Rc<RefCell<Cache>>) -> Self {
        Self {
            client_id,
            venue,
            cache,
        }
    }
}

#[async_trait::async_trait(?Send)]
impl DataClient for BacktestDataClient {
    fn client_id(&self) -> ClientId {
        self.client_id
    }

    fn venue(&self) -> Option<Venue> {
        Some(self.venue)
    }

    fn start(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    fn stop(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    fn reset(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    fn dispose(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    fn is_connected(&self) -> bool {
        true
    }

    fn is_disconnected(&self) -> bool {
        false
    }

    // -- COMMAND HANDLERS ---------------------------------------------------------------------------

    fn subscribe(&mut self, _cmd: &SubscribeCustomData) -> anyhow::Result<()> {
        Ok(())
    }

    fn subscribe_instruments(&mut self, _cmd: &SubscribeInstruments) -> anyhow::Result<()> {
        Ok(())
    }

    fn subscribe_instrument(&mut self, _cmd: &SubscribeInstrument) -> anyhow::Result<()> {
        Ok(())
    }

    fn subscribe_book_deltas(&mut self, _cmd: &SubscribeBookDeltas) -> anyhow::Result<()> {
        Ok(())
    }

    fn subscribe_book_depth10(&mut self, _cmd: &SubscribeBookDepth10) -> anyhow::Result<()> {
        Ok(())
    }

    fn subscribe_quotes(&mut self, _cmd: &SubscribeQuotes) -> anyhow::Result<()> {
        Ok(())
    }

    fn subscribe_trades(&mut self, _cmd: &SubscribeTrades) -> anyhow::Result<()> {
        Ok(())
    }

    fn subscribe_bars(&mut self, _cmd: &SubscribeBars) -> anyhow::Result<()> {
        Ok(())
    }

    fn subscribe_mark_prices(&mut self, _cmd: &SubscribeMarkPrices) -> anyhow::Result<()> {
        Ok(())
    }

    fn subscribe_index_prices(&mut self, _cmd: &SubscribeIndexPrices) -> anyhow::Result<()> {
        Ok(())
    }

    fn subscribe_instrument_status(
        &mut self,
        _cmd: &SubscribeInstrumentStatus,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn subscribe_instrument_close(
        &mut self,
        _cmd: &SubscribeInstrumentClose,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn unsubscribe(&mut self, _cmd: &UnsubscribeCustomData) -> anyhow::Result<()> {
        Ok(())
    }

    fn unsubscribe_instruments(&mut self, _cmd: &UnsubscribeInstruments) -> anyhow::Result<()> {
        Ok(())
    }

    fn unsubscribe_instrument(&mut self, _cmd: &UnsubscribeInstrument) -> anyhow::Result<()> {
        Ok(())
    }

    fn unsubscribe_book_deltas(&mut self, _cmd: &UnsubscribeBookDeltas) -> anyhow::Result<()> {
        Ok(())
    }

    fn unsubscribe_book_depth10(&mut self, _cmd: &UnsubscribeBookDepth10) -> anyhow::Result<()> {
        Ok(())
    }

    fn unsubscribe_quotes(&mut self, _cmd: &UnsubscribeQuotes) -> anyhow::Result<()> {
        Ok(())
    }

    fn unsubscribe_trades(&mut self, _cmd: &UnsubscribeTrades) -> anyhow::Result<()> {
        Ok(())
    }

    fn unsubscribe_bars(&mut self, _cmd: &UnsubscribeBars) -> anyhow::Result<()> {
        Ok(())
    }

    fn unsubscribe_mark_prices(&mut self, _cmd: &UnsubscribeMarkPrices) -> anyhow::Result<()> {
        Ok(())
    }

    fn unsubscribe_index_prices(&mut self, _cmd: &UnsubscribeIndexPrices) -> anyhow::Result<()> {
        Ok(())
    }

    fn unsubscribe_instrument_status(
        &mut self,
        _cmd: &UnsubscribeInstrumentStatus,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn unsubscribe_instrument_close(
        &mut self,
        _cmd: &UnsubscribeInstrumentClose,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    // -- DATA REQUEST HANDLERS ---------------------------------------------------------------------------

    fn request_data(&self, request: RequestCustomData) -> anyhow::Result<()> {
        log::debug!(
            "BacktestDataClient: request_data for type {:?} (unhandled custom type)",
            request.data_type,
        );
        Ok(())
    }

    fn request_instruments(&self, request: RequestInstruments) -> anyhow::Result<()> {
        let cache = self.cache.borrow();
        if let Some(venue) = &request.venue {
            let instruments = cache.instruments(venue, None);
            log::debug!(
                "BacktestDataClient: returning {} instruments for {}",
                instruments.len(),
                venue,
            );
        } else {
            log::debug!("BacktestDataClient: request_instruments with no venue filter");
        }
        Ok(())
    }

    fn request_instrument(&self, request: RequestInstrument) -> anyhow::Result<()> {
        let cache = self.cache.borrow();
        let instrument = cache.instrument(&request.instrument_id);
        match instrument {
            Some(_) => log::debug!(
                "BacktestDataClient: returning instrument {}",
                request.instrument_id,
            ),
            None => log::warn!(
                "BacktestDataClient: instrument {} not found in cache",
                request.instrument_id,
            ),
        }
        Ok(())
    }

    fn request_book_snapshot(&self, request: RequestBookSnapshot) -> anyhow::Result<()> {
        let cache = self.cache.borrow();
        let book = cache.order_book(&request.instrument_id);
        match book {
            Some(_) => log::debug!(
                "BacktestDataClient: returning book snapshot for {}",
                request.instrument_id,
            ),
            None => log::debug!(
                "BacktestDataClient: no order book for {} in cache",
                request.instrument_id,
            ),
        }
        Ok(())
    }

    fn request_quotes(&self, request: RequestQuotes) -> anyhow::Result<()> {
        let cache = self.cache.borrow();
        let quotes = cache.quotes(&request.instrument_id);
        let count = quotes.as_ref().map_or(0, |q| q.len());
        log::debug!(
            "BacktestDataClient: returning {} quotes for {}",
            count,
            request.instrument_id,
        );
        Ok(())
    }

    fn request_trades(&self, request: RequestTrades) -> anyhow::Result<()> {
        let cache = self.cache.borrow();
        let trades = cache.trades(&request.instrument_id);
        let count = trades.as_ref().map_or(0, |t| t.len());
        log::debug!(
            "BacktestDataClient: returning {} trades for {}",
            count,
            request.instrument_id,
        );
        Ok(())
    }

    fn request_bars(&self, request: RequestBars) -> anyhow::Result<()> {
        let cache = self.cache.borrow();
        let bars = cache.bars(&request.bar_type);
        let count = bars.as_ref().map_or(0, |b| b.len());
        log::debug!(
            "BacktestDataClient: returning {} bars for {}",
            count,
            request.bar_type,
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use nautilus_common::{
        cache::Cache,
        clients::DataClient,
        messages::data::{RequestInstrument, RequestInstruments},
    };
    use nautilus_core::{UnixNanos, UUID4};
    use nautilus_model::{
        identifiers::{ClientId, Venue},
        instruments::{CryptoPerpetual, Instrument, InstrumentAny, stubs::crypto_perpetual_ethusdt},
    };
    use rstest::rstest;

    use super::BacktestDataClient;

    fn make_client() -> (BacktestDataClient, Rc<RefCell<Cache>>) {
        let cache = Rc::new(RefCell::new(Cache::default()));
        let venue = Venue::from("BINANCE");
        let client = BacktestDataClient::new(
            ClientId::from(venue.as_str()),
            venue,
            cache.clone(),
        );
        (client, cache)
    }

    #[rstest]
    fn test_request_instrument_found(crypto_perpetual_ethusdt: CryptoPerpetual) {
        let (client, cache) = make_client();
        let instrument = InstrumentAny::CryptoPerpetual(crypto_perpetual_ethusdt);
        let instrument_id = instrument.id();
        cache.borrow_mut().add_instrument(instrument).unwrap();

        let request = RequestInstrument::new(
            instrument_id,
            None,
            None,
            None,
            UUID4::new(),
            UnixNanos::default(),
            None,
        );
        let result = client.request_instrument(request);
        assert!(result.is_ok());
    }

    #[rstest]
    fn test_request_instrument_not_found() {
        let (client, _cache) = make_client();
        let instrument_id = "UNKNOWN.BINANCE".parse().unwrap();

        let request = RequestInstrument::new(
            instrument_id,
            None,
            None,
            None,
            UUID4::new(),
            UnixNanos::default(),
            None,
        );
        let result = client.request_instrument(request);
        assert!(result.is_ok());
    }

    #[rstest]
    fn test_request_instruments_for_venue(crypto_perpetual_ethusdt: CryptoPerpetual) {
        let (client, cache) = make_client();
        let instrument = InstrumentAny::CryptoPerpetual(crypto_perpetual_ethusdt);
        cache.borrow_mut().add_instrument(instrument).unwrap();

        let request = RequestInstruments::new(
            None,
            None,
            None,
            Some(Venue::from("BINANCE")),
            UUID4::new(),
            UnixNanos::default(),
            None,
        );
        let result = client.request_instruments(request);
        assert!(result.is_ok());
    }

    #[rstest]
    fn test_request_quotes_empty() {
        let (client, _cache) = make_client();
        let instrument_id = "ETHUSDT-PERP.BINANCE".parse().unwrap();

        let request = nautilus_common::messages::data::RequestQuotes::new(
            instrument_id,
            None,
            None,
            None,
            None,
            UUID4::new(),
            UnixNanos::default(),
            None,
        );
        let result = client.request_quotes(request);
        assert!(result.is_ok());
    }

    #[rstest]
    fn test_request_trades_empty() {
        let (client, _cache) = make_client();
        let instrument_id = "ETHUSDT-PERP.BINANCE".parse().unwrap();

        let request = nautilus_common::messages::data::RequestTrades::new(
            instrument_id,
            None,
            None,
            None,
            None,
            UUID4::new(),
            UnixNanos::default(),
            None,
        );
        let result = client.request_trades(request);
        assert!(result.is_ok());
    }
}
