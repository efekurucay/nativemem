//! The nativemem hub: one local MCP server that holds the real MCP servers and their secrets and
//! exposes their tools to an agent. It is not a sync kind and does not touch the engine or the
//! ledger.

pub mod config;
pub mod connect;
pub mod error;
pub mod import;
pub mod secrets;
pub mod serve;

pub use config::{Auth, HubConfig, ServerSpec, Transport};
pub use error::{HubError, Result};
pub use secrets::{FileStore, KeychainStore, SecretStore};
