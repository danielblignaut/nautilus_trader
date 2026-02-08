// -------------------------------------------------------------------------------------------------
//  BoxedStrategy — concrete wrapper for `Box<dyn Strategy>`.
//
//  Allows adding a trait-object strategy to the BacktestEngine (which requires
//  `Strategy + Component + Debug + 'static`).
// -------------------------------------------------------------------------------------------------

use std::fmt::Debug;
use std::ops::{Deref, DerefMut};

use nautilus_common::actor::{DataActor, DataActorCore};
use nautilus_common::component::Component;

use super::{Strategy, StrategyCore};

/// Concrete wrapper that holds a `Box<dyn Strategy>` and delegates all
/// required trait implementations so it can be added to `BacktestEngine`.
pub struct BoxedStrategy(pub Box<dyn Strategy>);

impl Debug for BoxedStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BoxedStrategy { .. }")
    }
}

impl Deref for BoxedStrategy {
    type Target = DataActorCore;
    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl DerefMut for BoxedStrategy {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.deref_mut()
    }
}

impl DataActor for BoxedStrategy {
    fn on_start(&mut self) -> anyhow::Result<()> {
        // First, let the inner strategy run its on_start logic (logging, state init, etc.)
        // It should NOT call subscribe_trades directly (TypeId mismatch with BoxedStrategy).
        DataActor::on_start(&mut *self.0)?;

        // Now subscribe on behalf of the inner strategy using Self = BoxedStrategy,
        // so the handler's get_actor_unchecked::<BoxedStrategy> matches the registry.
        for id in self.0.trade_subscription_ids() {
            self.subscribe_trades(id, None, None);
        }
        for id in self.0.book_subscription_ids() {
            self.subscribe_book_deltas(
                id,
                nautilus_model::enums::BookType::L2_MBP,
                None,
                None,
                true,
                None,
            );
        }
        Ok(())
    }

    fn on_stop(&mut self) -> anyhow::Result<()> {
        DataActor::on_stop(&mut *self.0)
    }

    fn on_trade(&mut self, tick: &nautilus_model::data::trade::TradeTick) -> anyhow::Result<()> {
        self.0.on_trade(tick)
    }

    fn on_book_deltas(
        &mut self,
        deltas: &nautilus_model::data::OrderBookDeltas,
    ) -> anyhow::Result<()> {
        self.0.on_book_deltas(deltas)
    }

    fn on_order_filled(
        &mut self,
        event: &nautilus_model::events::OrderFilled,
    ) -> anyhow::Result<()> {
        self.0.on_order_filled(event)
    }

    fn on_order_canceled(
        &mut self,
        event: &nautilus_model::events::OrderCanceled,
    ) -> anyhow::Result<()> {
        self.0.on_order_canceled(event)
    }
}

impl Strategy for BoxedStrategy {
    fn core_mut(&mut self) -> &mut StrategyCore {
        self.0.core_mut()
    }
}
