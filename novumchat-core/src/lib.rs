//! Shared building blocks for NovumChat.
//!
//! This crate is compiled for both the native backend and the WebAssembly
//! frontend, so it only contains plain data types and pure logic:
//!
//! * [`config`]: the `config.toml` schema (providers, MCP servers, tools, search…)
//! * [`types`]: chats, messages, models and other API payloads
//! * [`events`]: events streamed from the backend to the UI
//! * [`rpc`]: RPC method names and request types shared by both sides
//! * [`permissions`]: tool risk classification and the permission policy
//! * [`markdown`] (feature `markdown`): Markdown + LaTeX + artifact rendering

pub mod config;
pub mod events;
pub mod permissions;
pub mod rpc;
pub mod types;
pub mod util;

#[cfg(feature = "markdown")]
pub mod markdown;

pub use config::AppConfig;
pub use events::AppEvent;
pub use types::*;

/// Product name shown in the UI and used for platform directories.
pub const APP_NAME: &str = "NovumChat";
/// Version of the shared crate (kept in lock-step with the workspace).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Name of the single event channel the backend emits on.
pub const EVENT_CHANNEL: &str = "novum://event";
