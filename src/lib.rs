#![allow(clippy::expect_fun_call)]
#![cfg_attr(feature = "nightly", feature(unstable_provider_api))]

#[cfg(all(target_family = "wasm", feature = "uniffi"))]
compile_error!("`wasm32` is not compatible with `uniffi`");

#[cfg(all(target_family = "wasm", feature = "server"))]
compile_error!("`wasm32` is not compatible with `server`");

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

pub use error::{InternalError, InternalSnafu};
pub use route::Route;
pub use transit_macros::{error, error_shard, oneof, record, route};
