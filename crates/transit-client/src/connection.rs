use std::{collections::HashMap, sync::Arc};

use async_lock::Semaphore;
use futures::{
    SinkExt,
    channel::{
        mpsc::{SendError, UnboundedSender},
        oneshot::{self, Canceled},
    },
    lock::Mutex,
};
use snafu::prelude::*;
use tracing::trace;
use transit_core::{
    frame::{self, FrameTooLong, MessageError, MessageId, MessageIdTail},
    route::RouteExt,
};
use transit_macros::core_error;
use transit_rt::cancel::CancellationToken;

#[core_error]
pub enum RequestError {
    #[snafu(display("tx was dropped"))]
    Tx { source: Canceled },

    #[snafu(display("send failed"))]
    Send { source: SendError },
}

#[core_error]
pub enum ConnectionError {
    #[snafu(display("{source}"), context(false))]
    FrameTooLong { source: FrameTooLong },

    #[snafu(display("decode failed"))]
    Decode { source: bitcode::Error },

    #[snafu(display("connection closed"))]
    Closed,

    #[snafu(display("request error"))]
    Request { source: RequestError },

    #[snafu(display("protocol error"))]
    Protocol { source: MessageError },
}

pub type Registry = HashMap<MessageIdTail, oneshot::Sender<Result<Vec<u8>, MessageError>>>;

pub(crate) struct Connection {
    pub(crate) write_tx: UnboundedSender<Vec<u8>>,
    pub(crate) registry: Arc<Mutex<Registry>>,
    pub(crate) closed: CancellationToken,
    pub(crate) permits: Semaphore,
}

impl Connection {
    pub(crate) async fn route<R: RouteExt>(
        &self,
        q: R::Request,
        id: MessageId,
    ) -> Result<R::Response, ConnectionError> {
        let data = bitcode::encode(&q);
        let buf = frame::qframe_encode(id, R::ID, data)?;
        let (tx, rx) = oneshot::channel();

        trace!(
            msgid = hex::encode(id.to_bytes()),
            route = hex::encode(R::ID.to_le_bytes()),
            "Sending request"
        );

        let res = self
            .closed
            .run_until_cancelled(async move {
                let _permit = self.permits.acquire().await;

                self.registry.lock().await.insert(id.tail(), tx);
                self.write_tx.clone().send(buf).await.context(SendSnafu)?;
                rx.await.context(TxSnafu)
            })
            .await
            .context(ClosedSnafu)?
            .context(RequestSnafu)?
            .context(ProtocolSnafu)?;

        let res = bitcode::decode(&res).context(DecodeSnafu)?;

        Ok(res)
    }

    pub(crate) async fn drop_tx(&self, id: MessageId) {
        self.registry.lock().await.remove(&id.tail());
    }
}
