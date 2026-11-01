use std::sync::Arc;

use futures::{AsyncRead, lock::Mutex};
use tracing::{debug, trace, warn};
use transit_core::frame::{self, *};
use transit_rt::{Runtime, cancel::CancellationToken, send::ArchSend};

use crate::connection::Registry;

pub(super) async fn pframe_decode_thread<Rt: Runtime, R: AsyncRead + Unpin + ArchSend>(
    mut reader: R,
    max_len: usize,
    tx: Arc<Mutex<Registry>>,
    notify: CancellationToken,
) {
    debug!("Frame decode thread started");

    let job = async {
        loop {
            let frame = match frame::next::<Rt, _>(&mut reader, max_len).await {
                Ok(frame) => frame,
                Err(err) => {
                    warn!(
                        "Error reading frame, closing connection. Full report:\n{}",
                        snafu::Report::from_error(err).to_string()
                    );
                    notify.cancel();
                    return;
                }
            };

            if frame.len() < MESSAGE_ID_LEN {
                warn!("Frame less than minimum size, ignoring");
                continue;
            }

            let mut tx = tx.lock().await;
            let msgid = MessageId::try_from(&frame[..MESSAGE_ID_LEN]).unwrap();
            let res = msgid.check().map(|_| frame[MESSAGE_ID_LEN..].to_vec());

            if msgid.goaway() && tx.is_empty() {
                debug!("Server sent goaway, new requests will reconnect");
                notify.cancel();
            }

            let Some(tx) = tx.remove(&msgid.tail()) else {
                debug!(
                    msgid = hex::encode(msgid.to_bytes()),
                    "Unknown message id, ignoring"
                );
                continue;
            };

            trace!(msgid = hex::encode(msgid.to_bytes()), "Received response");

            let Ok(_) = tx.send(res) else {
                warn!(
                    msgid = hex::encode(msgid.to_bytes()),
                    "rx dropped, ignoring"
                );
                continue;
            };
        }
    };

    if notify.run_until_cancelled(job).await.is_none() {
        warn!("Frame decode thread cancelled");
    }
}
