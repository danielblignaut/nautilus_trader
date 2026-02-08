//! Factory for creating Polymarket execution clients.

use std::{any::Any, cell::RefCell, rc::Rc};

use nautilus_common::cache::Cache;
use nautilus_common::clients::ExecutionClient;
use nautilus_model::identifiers::ClientId;
use nautilus_system::factories::{ClientConfig, ExecutionClientFactory};

use crate::{config::PolymarketExecutionClientConfig, execution::PolymarketExecutionClient};

impl ClientConfig for PolymarketExecutionClientConfig {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Factory for creating Polymarket execution clients.
#[derive(Debug, Default)]
pub struct PolymarketExecutionClientFactory;

impl PolymarketExecutionClientFactory {
    /// Creates a new [`PolymarketExecutionClientFactory`] instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl ExecutionClientFactory for PolymarketExecutionClientFactory {
    fn name(&self) -> &str {
        "PolymarketExecutionClientFactory"
    }

    fn config_type(&self) -> &str {
        "PolymarketExecutionClientConfig"
    }

    fn create(
        &self,
        name: &str,
        config: &dyn ClientConfig,
        cache: Rc<RefCell<Cache>>,
    ) -> anyhow::Result<Box<dyn ExecutionClient>> {
        let poly_config = config
            .as_any()
            .downcast_ref::<PolymarketExecutionClientConfig>()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Invalid config type for PolymarketExecutionClientFactory. \
                    Expected PolymarketExecutionClientConfig, was {config:?}",
                )
            })?;

        let client_id = ClientId::from(name);
        let client = PolymarketExecutionClient::new(client_id, poly_config.clone(), cache)?;

        Ok(Box::new(client))
    }
}
