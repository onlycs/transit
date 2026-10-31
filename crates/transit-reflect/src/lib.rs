#![cfg_attr(
    feature = "unstable",
    feature(
        type_info,
        const_cmp,
        const_trait_impl,
        const_index,
        rustc_attrs,
        f16,
        f128
    ),
    allow(internal_features)
)]

#[cfg(not(any(feature = "stable", feature = "unstable")))]
compile_error!("Must enable either (default) `stable` or `unstable`");

use crate::common::Hasher;

pub trait TypeHash {
    const HASH: u64;
}

mod common;
#[cfg_attr(feature = "unstable", path = "unstable.rs")]
#[cfg_attr(all(feature = "stable", not(feature = "unstable")), path = "stable.rs")]
mod imp;

pub const fn route_id<Q: TypeHash, P: TypeHash>(name: &str) -> u64 {
    let mut h = Hasher::new(0);
    h.next(name.as_bytes());
    h.next(&Q::HASH.to_le_bytes());
    h.next(&P::HASH.to_le_bytes());

    h.finish()
}
