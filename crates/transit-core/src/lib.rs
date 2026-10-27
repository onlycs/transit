#![cfg_attr(feature = "nightly", feature(error_generic_member_access))]

pub mod cancel;
pub mod error;
pub mod frame;
pub mod route;
pub mod send;

pub use error::private::{InternalError, InternalErrorCtx::InternalSnafu};
pub use route::Route;
pub use transit_macros::{constvar, error, error_shard, oneof, record, route};

#[cfg(all(
    target_family = "wasm",
    not(feature = "wasm"),
    not(feature = "futures-not-send")
))]
compile_error!("on wasm, enable `wasm` (JS host) and/or `futures-not-send`");

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!("transit_core");
