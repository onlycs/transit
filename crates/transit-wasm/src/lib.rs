pub mod client;
pub mod rt;

use std::{rc::Rc, time::Duration};

use transit_client::{TransitClient, TransitOptions};
use wasm_bindgen::prelude::*;

use crate::{client::TransitWsBrowser, rt::RtWasm};

type Client = TransitClient<RtWasm, TransitWsBrowser>;

#[wasm_bindgen]
pub struct Transit(Rc<Client>);

#[wasm_bindgen]
impl Transit {
    pub async fn connect(url: String, timeout_ms: u32) -> Result<Transit, JsError> {
        let client = Client::connect(TransitOptions {
            connect: url,
            timeout: Duration::from_millis(timeout_ms as u64),
        })
        .await
        .map_err(|e| JsError::new(&snafu::Report::from_error(&e).to_string()))?;
        Ok(Transit(Rc::new(client)))
    }
}

impl Transit {
    #[doc(hidden)]
    pub fn client(&self) -> Rc<Client> {
        Rc::clone(&self.0)
    }
}

#[derive(Debug, Clone, serde::Serialize, tsify::Tsify)]
pub struct TransitError {
    pub display: String,
    pub report: String,
}

impl TransitError {
    pub fn from_err<E: std::error::Error>(e: &E) -> Self {
        Self {
            display: e.to_string(),
            report: snafu::Report::from_error(e).to_string(),
        }
    }
}
impl std::fmt::Display for TransitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.display)
    }
}
impl std::error::Error for TransitError {}
