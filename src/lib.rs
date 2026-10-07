#![allow(clippy::expect_fun_call)]
#![cfg_attr(feature = "nightly", feature(error_generic_member_access))]

#[cfg(not(any(feature = "client", feature = "server")))]
compile_error!("must enable either `client` or `server` feature");

#[cfg(not(any(feature = "async-io", feature = "tokio")))]
compile_error!("must enable either `async-io` or `tokio` feature");

#[cfg(all(feature = "async-io", feature = "tokio"))]
compile_error!("cannot enable both `async-io` and `tokio` features");

#[cfg(all(target_family = "wasm", feature = "uniffi"))]
compile_error!("wasm is not compatible with `uniffi`");

#[cfg(all(target_family = "wasm", feature = "server"))]
compile_error!("wasm is not compatible with `server`");

#[cfg(all(target_family = "wasm", feature = "async-io"))]
compile_error!("wasm is not compatible with `async-io`");

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!("transit_core");

#[cfg(feature = "client")]
#[cfg_attr(target_family = "wasm", path = "wasm32.rs")]
#[cfg_attr(not(target_family = "wasm"), path = "tcp.rs")]
mod arch;

#[cfg(feature = "client")]
pub mod client;

#[cfg(feature = "server")]
pub mod server;

#[cfg(target_family = "wasm")]
pub mod wbg_util;

pub mod error;
pub mod frame;
pub mod route;
mod rt;

pub use error::private::{InternalError, InternalErrorCtx::InternalSnafu};
pub use route::Route;
pub use transit_macros::{error, error_shard, oneof, record, route};

#[cfg(feature = "uniffi")]
#[derive(uniffi::Enum)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[cfg(feature = "uniffi")]
#[uniffi::export]
pub fn begin_logging(level: LogLevel) {
    use tracing_subscriber::{filter::Targets, layer::SubscriberExt, util::SubscriberInitExt};

    let level = match level {
        LogLevel::Trace => tracing::Level::TRACE,
        LogLevel::Debug => tracing::Level::DEBUG,
        LogLevel::Info => tracing::Level::INFO,
        LogLevel::Warn => tracing::Level::WARN,
        LogLevel::Error => tracing::Level::ERROR,
    };

    let fmt = tracing_subscriber::fmt::layer().pretty();
    let filter = Targets::new().with_default(level);

    tracing_subscriber::registry().with(filter).with(fmt).init();
}
