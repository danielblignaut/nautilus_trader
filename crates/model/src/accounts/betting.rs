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

//! A betting account for binary outcome / prediction market instruments.

use std::{
    fmt::Display,
    ops::{Deref, DerefMut},
};

use ahash::AHashMap;
use serde::{Deserialize, Serialize};

use crate::{
    accounts::{Account, base::BaseAccount},
    enums::{AccountType, LiquiditySide, OrderSide},
    events::{AccountState, OrderFilled},
    identifiers::{AccountId, InstrumentId},
    instruments::InstrumentAny,
    position::Position,
    types::{AccountBalance, Currency, Money, Price, Quantity, money::MoneyRaw},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(module = "nautilus_trader.core.nautilus_pyo3.model")
)]
pub struct BettingAccount {
    pub base: BaseAccount,
    pub allow_borrowing: bool,
    #[serde(skip, default)]
    pub balances_locked: AHashMap<(InstrumentId, Currency), Money>,
}

impl BettingAccount {
    /// Creates a new [`BettingAccount`] instance.
    #[must_use]
    pub fn new(event: AccountState, calculate_account_state: bool, allow_borrowing: bool) -> Self {
        Self {
            base: BaseAccount::new(event, calculate_account_state),
            allow_borrowing,
            balances_locked: AHashMap::new(),
        }
    }

    /// Updates the locked balance for the given instrument and currency.
    pub fn update_balance_locked(&mut self, instrument_id: InstrumentId, locked: Money) {
        assert!(locked.raw >= 0, "locked balance was negative: {locked}");
        let currency = locked.currency;
        self.balances_locked
            .insert((instrument_id, currency), locked);
        self.recalculate_balance(currency);
    }

    /// Clears all locked balances for the given instrument ID.
    pub fn clear_balance_locked(&mut self, instrument_id: InstrumentId) {
        let currencies_to_recalc: Vec<Currency> = self
            .balances_locked
            .keys()
            .filter(|(id, _)| *id == instrument_id)
            .map(|(_, currency)| *currency)
            .collect();

        for currency in &currencies_to_recalc {
            self.balances_locked.remove(&(instrument_id, *currency));
        }

        for currency in currencies_to_recalc {
            self.recalculate_balance(currency);
        }
    }

    /// Updates the account balances, enforcing borrowing constraints.
    ///
    /// # Errors
    ///
    /// Returns an error if `allow_borrowing` is false and any balance has a negative total.
    pub fn update_balances(&mut self, balances: &[AccountBalance]) -> anyhow::Result<()> {
        if !self.allow_borrowing {
            for balance in balances {
                if balance.total.raw < 0 {
                    anyhow::bail!(
                        "Betting account balance would become negative: {} {} (borrowing not allowed for {})",
                        balance.total.as_decimal(),
                        balance.currency.code,
                        self.id
                    );
                }
            }
        }
        self.base.update_balances(balances);
        Ok(())
    }

    /// Recalculates the account balance for the specified currency based on per-instrument locks.
    pub fn recalculate_balance(&mut self, currency: Currency) {
        let current_balance = match self.balances.get(&currency) {
            Some(balance) => *balance,
            None => {
                log::debug!("Cannot recalculate balance when no current balance for {currency}");
                return;
            }
        };

        let total_locked_raw: MoneyRaw = self
            .balances_locked
            .values()
            .filter(|locked| locked.currency == currency)
            .map(|locked| locked.raw)
            .fold(0, |acc, raw| acc.saturating_add(raw));

        let total_raw = current_balance.total.raw;

        let (locked_raw, free_raw) = if total_locked_raw > total_raw && total_raw >= 0 {
            (total_raw, 0)
        } else {
            (total_locked_raw, total_raw - total_locked_raw)
        };

        let new_balance = AccountBalance::new(
            current_balance.total,
            Money::from_raw(locked_raw, currency),
            Money::from_raw(free_raw, currency),
        );

        self.balances.insert(currency, new_balance);
    }
}

impl Account for BettingAccount {
    fn id(&self) -> AccountId {
        self.id
    }

    fn account_type(&self) -> AccountType {
        self.account_type
    }

    fn base_currency(&self) -> Option<Currency> {
        self.base_currency
    }

    fn is_cash_account(&self) -> bool {
        false
    }

    fn is_margin_account(&self) -> bool {
        false
    }

    fn calculated_account_state(&self) -> bool {
        false
    }

    fn balance_total(&self, currency: Option<Currency>) -> Option<Money> {
        self.base_balance_total(currency)
    }

    fn balances_total(&self) -> AHashMap<Currency, Money> {
        self.base_balances_total()
    }

    fn balance_free(&self, currency: Option<Currency>) -> Option<Money> {
        self.base_balance_free(currency)
    }

    fn balances_free(&self) -> AHashMap<Currency, Money> {
        self.base_balances_free()
    }

    fn balance_locked(&self, currency: Option<Currency>) -> Option<Money> {
        self.base_balance_locked(currency)
    }

    fn balances_locked(&self) -> AHashMap<Currency, Money> {
        self.base_balances_locked()
    }

    fn balance(&self, currency: Option<Currency>) -> Option<&AccountBalance> {
        self.base_balance(currency)
    }

    fn last_event(&self) -> Option<AccountState> {
        self.base_last_event()
    }

    fn events(&self) -> Vec<AccountState> {
        self.events.clone()
    }

    fn event_count(&self) -> usize {
        self.events.len()
    }

    fn currencies(&self) -> Vec<Currency> {
        self.balances.keys().copied().collect()
    }

    fn starting_balances(&self) -> AHashMap<Currency, Money> {
        self.balances_starting.clone()
    }

    fn balances(&self) -> AHashMap<Currency, AccountBalance> {
        self.balances.clone()
    }

    fn apply(&mut self, event: AccountState) -> anyhow::Result<()> {
        if !self.allow_borrowing {
            for balance in &event.balances {
                if balance.total.raw < 0 {
                    anyhow::bail!(
                        "Cannot apply account state: balance would be negative {} {} \
                        (borrowing not allowed for {})",
                        balance.total.as_decimal(),
                        balance.currency.code,
                        self.id
                    );
                }
            }
        }

        self.balances_locked.clear();
        self.base_apply(event);
        Ok(())
    }

    fn purge_account_events(&mut self, ts_now: nautilus_core::UnixNanos, lookback_secs: u64) {
        self.base.base_purge_account_events(ts_now, lookback_secs);
    }

    fn calculate_balance_locked(
        &mut self,
        instrument: InstrumentAny,
        side: OrderSide,
        quantity: Quantity,
        price: Price,
        use_quote_for_inverse: Option<bool>,
    ) -> anyhow::Result<Money> {
        self.base_calculate_balance_locked(instrument, side, quantity, price, use_quote_for_inverse)
    }

    fn calculate_pnls(
        &self,
        _instrument: InstrumentAny,
        fill: OrderFilled,
        position: Option<Position>,
    ) -> anyhow::Result<Vec<Money>> {
        // Betting-specific PnL: realized as (exit_price - entry_price) * quantity
        let mut pnls: Vec<Money> = Vec::new();

        if let Some(ref pos) = position
            && pos.quantity.is_positive()
            && pos.entry != fill.order_side
        {
            let pnl_quantity = Quantity::from_raw(
                fill.last_qty.raw.min(pos.quantity.raw),
                fill.last_qty.precision,
            );

            let pnl = pos.calculate_pnl(pos.avg_px_open, fill.last_px.as_f64(), pnl_quantity);

            pnls.push(pnl);
        }

        Ok(pnls)
    }

    fn calculate_commission(
        &self,
        instrument: InstrumentAny,
        last_qty: Quantity,
        last_px: Price,
        liquidity_side: LiquiditySide,
        use_quote_for_inverse: Option<bool>,
    ) -> anyhow::Result<Money> {
        self.base_calculate_commission(
            instrument,
            last_qty,
            last_px,
            liquidity_side,
            use_quote_for_inverse,
        )
    }
}

impl Deref for BettingAccount {
    type Target = BaseAccount;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl DerefMut for BettingAccount {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

impl PartialEq for BettingAccount {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for BettingAccount {}

impl Display for BettingAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "BettingAccount(id={}, type={}, base={})",
            self.id,
            self.account_type,
            self.base_currency.map_or_else(
                || "None".to_string(),
                |base_currency| format!("{}", base_currency.code)
            ),
        )
    }
}
