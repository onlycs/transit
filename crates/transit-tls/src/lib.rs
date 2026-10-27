#![cfg_attr(feature = "nightly", feature(error_generic_member_access))]

pub struct TransitTls<I>(std::marker::PhantomData<fn(I)>);

#[cfg(feature = "client")]
pub mod client;

#[cfg(feature = "server")]
pub mod server;
