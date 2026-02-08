//! Polymarket execution adapter for the Nautilus trading engine.
//!
//! Provides a Rust implementation of the Polymarket CLOB execution client,
//! including EIP-712 order signing, REST API order management, and WebSocket
//! real-time order/trade updates.
//!
//! # Architecture
//!
//! - `config`: `PolymarketExecutionClientConfig` — API credentials, URLs, private key
//! - `factory`: `PolymarketExecutionClientFactory` impl `ExecutionClientFactory`
//! - `execution`: `PolymarketExecutionClient` impl `ExecutionClient`
//! - `types`: API request/response DTOs
//! - `signing`: EIP-712 order signing for Polymarket's CTF Exchange
//! - `websocket`: WebSocket connection management and message parsing

#![warn(rustc::all)]
#![deny(unsafe_code)]
#![deny(nonstandard_style)]

pub mod config;
pub mod execution;
pub mod factory;
pub mod signing;
pub mod types;
pub mod websocket;

pub use crate::{
    config::{OrderResultMap, PolymarketExecutionClientConfig, TokenMap},
    execution::PolymarketExecutionClient,
    factory::PolymarketExecutionClientFactory,
};
