use std::cell::RefCell;

use wasm_bindgen::{JsValue, prelude::*};

use crate::client::RouteErrorWrapped;

#[wasm_bindgen(inline_js = r#"
export function __transit_identity(x) {
    return x;
}
"#)]
extern "C" {
    fn __transit_identity(x: RouteErrorWrapped) -> JsValue;
}

pub fn serialize_wrapper<S: serde::Serializer>(
    val: &RefCell<Option<RouteErrorWrapped>>,
    ser: S,
) -> Result<S::Ok, S::Error> {
    // It's responsibility of serde-wasm-bindgen's Serializer to clone the
    // value. For all other serializers, using reference instead of cloning
    // here will ensure that we don't create accidental leaks.

    // dont know what that^ means, but i do know i shouldn't have had to write
    // this.
    let own = val
        .borrow_mut()
        .take()
        .expect("serialize_wrapper: value already serialized");

    let jsv = __transit_identity(own);

    serde_wasm_bindgen::preserve::serialize(&jsv, ser)
}
