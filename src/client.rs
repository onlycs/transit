use std::{collections::HashMap, sync::Arc, time::Duration};

use snafu::{Location, prelude::*};
use tokio::{
    self,
    sync::{
        Mutex,
        mpsc::{self, UnboundedSender, error::SendError},
        oneshot::{self, error::RecvError},
    },
};
use tokio_util::sync::CancellationToken;
use transit_macros::core_error;
#[cfg(target_family = "wasm")]
use wasm_bindgen::prelude::*;

use crate::{
    InternalError, Route,
    arch::{self, *},
    frame::{self, MessageId, RouteId},
};

#[core_error]
pub enum RouteError {
    #[snafu(display("Frame error"))]
    Frame {
        source: frame::FrameError,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("TX was dropped"))]
    Tx {
        source: RecvError,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Decode failed"))]
    Decode {
        source: bitcode::Error,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Connection failed"))]
    Connect {
        source: arch::ConnectError,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Internal error while fetching known routes"))]
    FetchKnownRoutes {
        source: InternalError,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Send failed"))]
    Send {
        source: SendError<Vec<u8>>,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Request timeout"))]
    Timeout {
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Request closed"))]
    Closed {
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Unknown route {}", hex::encode(route.to_le_bytes())))]
    UnknownRoute {
        route: RouteId,
        #[snafu(implicit)]
        location: Location,
    },

    #[cfg(target_family = "wasm")]
    #[snafu(display("Invalid request: {message}"))]
    InvalidRequest { message: String },

    #[cfg(target_family = "wasm")]
    #[snafu(display("Response serialization failed: {message}"))]
    Serialization { message: String },
}

#[derive(Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[cfg_attr(target_family = "wasm", wasm_bindgen)]
pub struct TransitOptions {
    #[cfg_attr(target_family = "wasm", wasm_bindgen(getter_with_clone))]
    pub connect: ConnectOptions,
    pub timeout_ms: u64,
}

#[cfg(target_family = "wasm")]
#[wasm_bindgen]
impl TransitOptions {
    #[wasm_bindgen(constructor)]
    pub fn new(connect: ConnectOptions) -> Self {
        Self {
            connect,
            timeout_ms: 60 * 1000, // 60s should be fine(TM)
        }
    }

    #[wasm_bindgen(js_name = "withTimeout")]
    pub fn new_with_timeout(connect: ConnectOptions, timeout_ms: u64) -> Self {
        Self {
            connect,
            timeout_ms,
        }
    }
}

pub type Registry = HashMap<MessageId, oneshot::Sender<Option<Vec<u8>>>>;

struct Connection {
    _read: arch::JoinHandle<()>,
    _write: arch::JoinHandle<()>,
    write_tx: UnboundedSender<Vec<u8>>,

    registry: Arc<Mutex<Registry>>,
    closed: CancellationToken,
}

impl Connection {
    async fn route<R: Route>(
        &self,
        q: R::Request,
        id: MessageId,
    ) -> Result<R::Response, RouteError> {
        let data = bitcode::encode(&q);
        let buf = frame::qframe_encode(id, R::ID, data).context(FrameSnafu)?;
        let (tx, rx) = oneshot::channel();

        let res = self
            .closed
            .run_until_cancelled(async move {
                self.registry.lock().await.insert(id, tx);
                self.write_tx.send(buf).context(SendSnafu)?;
                rx.await.context(TxSnafu) // this is generally what is waited on, but wrap everything
            })
            .await
            .ok_or_else(|| ClosedSnafu.build())??
            .context(UnknownRouteSnafu { route: R::ID })?;

        let res = bitcode::decode(&res).context(DecodeSnafu)?;

        Ok(res)
    }

    async fn drop_tx(&self, id: MessageId) {
        self.registry.lock().await.remove(&id);
    }
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
#[cfg_attr(target_family = "wasm", wasm_bindgen::prelude::wasm_bindgen)]
pub struct Transit {
    connection: Mutex<Arc<Connection>>,
    options: TransitOptions,
}

impl Transit {
    async fn connection(&self) -> Result<Arc<Connection>, RouteError> {
        let mut conn = self.connection.lock().await;

        if conn.closed.is_cancelled() {
            *conn = Arc::new(_connect(&self.options).await.context(ConnectSnafu)?);
        }

        Ok(Arc::clone(&conn))
    }

    pub async fn route<R: Route>(&self, q: R::Request) -> Result<R::Response, RouteError> {
        let conn = self.connection().await?;
        let id = frame::gen_msgid().context(FrameSnafu)?;
        let timeout = Duration::from_millis(self.options.timeout_ms);

        match arch::timeout(timeout, conn.route::<R>(q, id))
            .await
            .ok_or_else(|| TimeoutSnafu.build())
            .flatten()
        {
            Ok(data) => Ok(data),
            Err(err) => {
                conn.drop_tx(id).await;
                Err(err)
            }
        }
    }
}

async fn _connect(options: &TransitOptions) -> Result<Connection, ConnectError> {
    let registry = Arc::new(Mutex::new(Registry::default()));
    let notify = CancellationToken::default();
    let (read, write) = arch::connect(&options.connect).await?;
    let (tx, rx) = mpsc::unbounded_channel();

    let rt = arch::spawn(frame::pframe_deocde_thread(
        read,
        Arc::clone(&registry),
        notify.clone(),
    ));

    let wt = arch::spawn(frame::frame_encode_thread(write, rx, notify.clone()));

    Ok(Connection {
        _read: rt,
        _write: wt,
        write_tx: tx,

        registry,
        closed: notify,
    })
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
#[cfg_attr(target_family = "wasm", wasm_bindgen::prelude::wasm_bindgen)]
#[cfg_attr(target_family = "wasm", allow(clippy::arc_with_non_send_sync))]
pub async fn connect(options: TransitOptions) -> Result<Transit, ConnectError> {
    Ok(Transit {
        connection: Mutex::new(Arc::new(_connect(&options).await?)),
        options,
    })
}
