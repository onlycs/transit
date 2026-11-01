#![cfg_attr(feature = "nightly", feature(error_generic_member_access))]

pub mod connection;
mod threads;
pub mod transport;

use std::{marker, sync::Arc, time::Duration};

use async_lock::Semaphore;
use futures::{AsyncReadExt, StreamExt, channel::mpsc, lock::Mutex};
use snafu::{AsErrorSource, prelude::*};
use tracing::{debug, info};
use transit_core::{
    Route,
    frame::{self, FrameLen, MAX_ALLOWED_FRAME_LEN, MIN_FRAME_LEN, MessageId},
};
use transit_macros::core_error;
use transit_rt::{Runtime, cancel::CancellationToken};

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
pub struct ClientOptions {
    pub timeout: Duration,
    pub max_in_flight: usize,
    pub max_response_len: usize,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(60),
            max_in_flight: 64,
            max_response_len: 5 * 1024 * 1024,
        }
    }
}

pub struct TransitClient<Rt: Runtime, Tr: ClientTransport> {
    connection: Mutex<Arc<Connection>>,
    connect_opts: Tr::ConnectOptions,
    client_opts: ClientOptions,
    _rt: marker::PhantomData<Rt>,
}

impl<Rt: Runtime, Tr: ClientTransport> TransitClient<Rt, Tr> {
    async fn connection(&self) -> Result<Arc<Connection>, Tr::ConnectError> {
        let mut conn = self.connection.lock().await;

        if conn.closed.is_cancelled() {
            info!("Connection closed, reconnecting");
            *conn = Arc::new(Self::_connect(&self.connect_opts, &self.client_opts).await?);
        }

        Ok(Arc::clone(&conn))
    }

    async fn _connect(
        connect: &Tr::ConnectOptions,
        client: &ClientOptions,
    ) -> Result<Connection, Tr::ConnectError> {
        let registry = Arc::new(Mutex::new(Registry::default()));
        let notify = CancellationToken::default();
        let (read, write) = Tr::connect::<Rt>(connect.clone()).await?.split();
        let (tx, rx) = mpsc::unbounded::<Vec<u8>>();

        let rx = Box::pin(rx.filter({
            let registry = Arc::clone(&registry);

            move |buf| {
                let registry = Arc::clone(&registry);
                let msgid = buf
                    .get(size_of::<FrameLen>()..size_of::<FrameLen>() + frame::MESSAGE_ID_LEN)
                    .and_then(|bytes| MessageId::try_from(bytes).ok());

                async move {
                    match msgid {
                        Some(id) => registry.lock().await.contains_key(&id.tail()),
                        None => true,
                    }
                }
            }
        }));

        Rt::spawn(threads::pframe_decode_thread::<Rt, _>(
            read,
            client.max_response_len,
            Arc::clone(&registry),
            notify.clone(),
        ));

        Rt::spawn(frame::encode_thread::<Rt, _, _>(write, rx, notify.clone()));

        debug!("Connection established");

        Ok(Connection {
            write_tx: tx,
            registry,
            closed: notify,
            permits: Semaphore::new(client.max_in_flight),
        })
    }

    pub async fn route<R: Route>(
        &self,
        q: R::Request,
    ) -> Result<R::Response, RouteError<Tr::ConnectError>> {
        let conn = self.connection().await.context(ConnectSnafu)?;
        let id = MessageId::generate().context(MessageIdSnafu)?;
        let timeout = self.client_opts.timeout;

        match transit_rt::timeout::<Rt, _>(timeout, conn.route::<R>(q, id)).await {
            Some(Ok(data)) => Ok(data),
            Some(Err(err)) => {
                debug!(msgid = hex::encode(id.to_bytes()), %err, "Request failed");
                conn.drop_tx(id).await;
                Err(RouteError::from(err))
            }
            None => {
                debug!(msgid = hex::encode(id.to_bytes()), "Request timed out");
                conn.drop_tx(id).await;
                Err(TimeoutSnafu.build())
            }
        }
    }

    pub async fn connect(
        connect: Tr::ConnectOptions,
        client: ClientOptions,
    ) -> Result<Self, Tr::ConnectError> {
        assert!(
            client.max_response_len >= MIN_FRAME_LEN,
            "Max response len is too short, must be at least {MIN_FRAME_LEN}"
        );

        assert!(
            client.max_response_len <= MAX_ALLOWED_FRAME_LEN,
            "Max response len is too long, can be at most {MAX_ALLOWED_FRAME_LEN}"
        );

        Ok(Self {
            connection: Mutex::new(Arc::new(Self::_connect(&connect, &client).await?)),
            connect_opts: connect,
            client_opts: client,
            _rt: marker::PhantomData,
        })
    }
}
