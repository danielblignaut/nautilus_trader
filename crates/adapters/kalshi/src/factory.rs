//! Factory for creating Kalshi execution clients.

use std::{any::Any, cell::RefCell, rc::Rc};

use nautilus_common::cache::Cache;
use nautilus_common::clients::ExecutionClient;
use nautilus_model::identifiers::ClientId;
use nautilus_system::factories::{ClientConfig, ExecutionClientFactory};

use crate::{config::KalshiExecutionClientConfig, execution::KalshiExecutionClient};

impl ClientConfig for KalshiExecutionClientConfig {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Factory for creating Kalshi execution clients.
#[derive(Debug, Default)]
pub struct KalshiExecutionClientFactory;

impl KalshiExecutionClientFactory {
    /// Creates a new [`KalshiExecutionClientFactory`] instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl ExecutionClientFactory for KalshiExecutionClientFactory {
    fn name(&self) -> &str {
        "KalshiExecutionClientFactory"
    }

    fn config_type(&self) -> &str {
        "KalshiExecutionClientConfig"
    }

    fn create(
        &self,
        name: &str,
        config: &dyn ClientConfig,
        cache: Rc<RefCell<Cache>>,
    ) -> anyhow::Result<Box<dyn ExecutionClient>> {
        let kalshi_config = config
            .as_any()
            .downcast_ref::<KalshiExecutionClientConfig>()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Invalid config type for KalshiExecutionClientFactory. \
                    Expected KalshiExecutionClientConfig, was {config:?}",
                )
            })?;

        let client_id = ClientId::from(name);
        let client = KalshiExecutionClient::new(client_id, kalshi_config.clone(), cache)?;

        Ok(Box::new(client))
    }
}
