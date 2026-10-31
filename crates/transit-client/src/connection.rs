use std::{collections::HashMap, sync::Arc};

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
    frame::{self, FrameTooLong, MessageId, RouteId},
    route::RouteExt,
};
use transit_macros::core_error;
use transit_rt::cancel::CancellationToken;

use crate::threads::ProtocolError;

#[core_error]
pub enum ConnectionError {
    #[snafu(display("{source}"), context(false))]
    FrameTooLong { source: FrameTooLong },

    #[snafu(display("tx was dropped"))]
    Tx { source: Canceled },

    #[snafu(display("decode failed"))]
    Decode { source: bitcode::Error },

    #[snafu(display("send failed"))]
    Send { source: SendError },

    #[snafu(display("request timeout"))]
    Timeout,

    #[snafu(display("request closed"))]
    Closed,

    #[snafu(display("unknown route {}", hex::encode(route.to_le_bytes())))]
    UnknownRoute { route: RouteId },

    #[snafu(display("server did not handle route id check"))]
    NoRouteIdCheck,

    #[snafu(display("server handled route id check incorrectly"))]
    InvalidRouteIdCheck { source: bitcode::Error },

    #[snafu(display("protocol error"))]
    Protocol { source: ProtocolError },
}

pub type Registry = HashMap<MessageId, oneshot::Sender<Result<Vec<u8>, ProtocolError>>>;

pub(crate) struct Connection {
    pub(crate) write_tx: UnboundedSender<Vec<u8>>,
    pub(crate) registry: Arc<Mutex<Registry>>,
    pub(crate) closed: CancellationToken,
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
                self.registry.lock().await.insert(id, tx);
                self.write_tx.clone().send(buf).await.context(SendSnafu)?;
                rx.await.context(TxSnafu)
            })
            .await
            .context(ClosedSnafu)??
            .context(ProtocolSnafu)?;

        let res = bitcode::decode(&res).context(DecodeSnafu)?;

        Ok(res)
    }

    pub(crate) async fn drop_tx(&self, id: MessageId) {
        self.registry.lock().await.remove(&id);
    }
}
