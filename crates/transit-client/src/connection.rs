use std::{collections::HashMap, sync::Arc};

use futures::{
    SinkExt,
    channel::{
        mpsc::{SendError, UnboundedSender},
        oneshot::{self, Canceled},
    },
    lock::Mutex,
};
use snafu::{OptionExt, ResultExt};
use tracing::trace;
use transit_core::{
    Route,
    cancel::CancellationToken,
    frame::{self, FrameTooLong, MessageId, RouteId},
};
use transit_macros::core_error;

#[core_error]
pub enum ConnectionError {
    #[snafu(display("{source}"), context(false))]
    FrameTooLong { source: FrameTooLong },

    #[snafu(display("tX was dropped"))]
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
}

pub type Registry = HashMap<MessageId, oneshot::Sender<Option<Vec<u8>>>>;

pub(crate) struct Connection {
    pub(crate) write_tx: UnboundedSender<Vec<u8>>,
    pub(crate) registry: Arc<Mutex<Registry>>,
    pub(crate) closed: CancellationToken,
}

impl Connection {
    pub(crate) async fn route<R: Route>(
        &self,
        q: R::Request,
        id: MessageId,
    ) -> Result<R::Response, ConnectionError> {
        let data = bitcode::encode(&q);
        let buf = frame::qframe_encode(id, R::ID, data)?;
        let (tx, rx) = oneshot::channel();

        trace!(
            msgid = hex::encode(id),
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
            .ok_or_else(|| ClosedSnafu.build())??
            .context(UnknownRouteSnafu { route: R::ID })?;

        let res = bitcode::decode(&res).context(DecodeSnafu)?;

        Ok(res)
    }

    pub(crate) async fn drop_tx(&self, id: MessageId) {
        self.registry.lock().await.remove(&id);
    }
}
