//! Kalshi execution adapter for the Nautilus trading engine.
//!
//! Provides a Rust implementation of the Kalshi CLOB execution client,
//! including RSA-PSS order signing, REST API order management, and WebSocket
//! real-time fill/order updates.
//!
//! # Architecture
//!
//! - `auth`: `KalshiAuth` — RSA-PSS signing for API authentication
//! - `config`: `KalshiExecutionClientConfig` — API credentials, URLs, subaccount
//! - `factory`: `KalshiExecutionClientFactory` impl `ExecutionClientFactory`
//! - `execution`: `KalshiExecutionClient` impl `ExecutionClient`
//! - `types`: API request/response DTOs
//! - `websocket`: WebSocket connection for fill/order/orderbook channels

#![warn(rustc::all)]
#![deny(unsafe_code)]
#![deny(nonstandard_style)]

pub mod auth;
pub mod config;
pub mod execution;
pub mod factory;
pub mod types;
pub mod websocket;

pub use crate::{
    auth::KalshiAuth,
    config::{KalshiExecutionClientConfig, OrderResultMap, TickerMap},
    execution::KalshiExecutionClient,
    factory::KalshiExecutionClientFactory,
};
