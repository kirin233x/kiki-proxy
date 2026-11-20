//! kiki-proxy 的网关引擎，不依赖 Tauri，可以单独测试。

pub mod auth;
pub mod clients;
pub mod config;
pub mod gateway;
pub mod ir;
pub mod plugin;
pub mod plugins;
pub mod pricing;
pub mod protocol;
pub mod router;
pub mod sse;
pub mod store;
pub mod upstream;

pub use gateway::Gateway;
