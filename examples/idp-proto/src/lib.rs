#[cfg(all(target_family = "wasm", feature = "uniffi"))]
compile_error!("`wasm32` is not compatible with `uniffi`");

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!("idp_proto");

pub mod auth;
pub mod error;
pub mod user;
