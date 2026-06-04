//! Protocol-agnostic core business logic.
//!
//! [`service::CoreService`] sits between transport layers (MCP, ConnectRPC)
//! and persistence layers ([`MemoryService`], [`IndexService`]).
//! It handles validation, orchestration, and structured returns.
//! Transport-specific formatting lives in the respective adapter crates.

pub mod service;
