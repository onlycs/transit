#![cfg_attr(feature = "nightly", feature(error_generic_member_access))]

pub mod connection;
mod threads;
pub mod transport;

use std::{marker, sync::Arc, time::Duration};

use futures::{AsyncReadExt, channel::mpsc, lock::Mutex};
use snafu::{AsErrorSource, ResultExt};
use tracing::{debug, info};
use transit_core::{
    Route,
    cancel::CancellationToken,
    frame::{self},
};
use transit_macros::core_error;
use transit_rt::Runtime;

use crate::{
    connection::{Connection, ConnectionError, Registry},
    transport::ClientTransport,
};

#[core_error]
pub enum RouteError<E: AsErrorSource> {
    #[snafu(display("{source}"), context(false))]
    Inner { source: ConnectionError },

    #[snafu(display("connection failed"))]
    Connect { source: E },

    #[snafu(display("failed to generate message id"))]
    MessageId { source: getrandom::Error },

    #[snafu(display("timed out"))]
    Timeout,
}

#[derive(Clone)]
pub struct TransitOptions<Tr: ClientTransport> {
    pub connect: Tr::ConnectOptions,
    pub timeout: Duration,
}

impl<Tr: ClientTransport> TransitOptions<Tr> {
    pub fn new(connect: Tr::ConnectOptions) -> Self {
        Self {
            connect,
            timeout: Duration::from_secs(60),
        }
    }
}

pub struct TransitClient<Rt: Runtime, Tr: ClientTransport> {
    connection: Mutex<Arc<Connection>>,
    options: TransitOptions<Tr>,
    _rt: marker::PhantomData<Rt>,
}

impl<Rt: Runtime, Tr: ClientTransport> TransitClient<Rt, Tr> {
    async fn connection(&self) -> Result<Arc<Connection>, Tr::ConnectError> {
        let mut conn = self.connection.lock().await;

        if conn.closed.is_cancelled() {
            info!("Connection closed, reconnecting");
            *conn = Arc::new(Self::_connect(&self.options).await?);
        }

        Ok(Arc::clone(&conn))
    }

    async fn _connect(options: &TransitOptions<Tr>) -> Result<Connection, Tr::ConnectError> {
        let registry = Arc::new(Mutex::new(Registry::default()));
        let notify = CancellationToken::default();
        let (read, write) = Tr::connect::<Rt>(options.connect.clone()).await?.split();
        let (tx, rx) = mpsc::unbounded();

        Rt::spawn(threads::pframe_decode_thread(
            read,
            Arc::clone(&registry),
            notify.clone(),
        ));

        Rt::spawn(transit_rt::frame_encode_thread::<Rt, _>(
            write,
            rx,
            notify.clone(),
        ));

        debug!("Connection established");

        Ok(Connection {
            write_tx: tx,
            registry,
            closed: notify,
        })
    }

    pub async fn route<R: Route>(
        &self,
        q: R::Request,
    ) -> Result<R::Response, RouteError<Tr::ConnectError>> {
        let conn = self.connection().await.context(ConnectSnafu)?;
        let id = frame::gen_msgid().context(MessageIdSnafu)?;
        let timeout = self.options.timeout;

        match transit_rt::timeout::<Rt, _>(timeout, conn.route::<R>(q, id)).await {
            Some(Ok(data)) => Ok(data),
            Some(Err(err)) => {
                debug!(msgid = hex::encode(id), %err, "Request failed");
                conn.drop_tx(id).await;
                Err(RouteError::from(err))
            }
            None => {
                debug!(msgid = hex::encode(id), "Request timed out");
                conn.drop_tx(id).await;
                Err(TimeoutSnafu.build())
            }
        }
    }

    pub async fn connect(options: TransitOptions<Tr>) -> Result<Self, Tr::ConnectError> {
        Ok(Self {
            connection: Mutex::new(Arc::new(Self::_connect(&options).await?)),
            options,
            _rt: marker::PhantomData,
        })
    }
}
