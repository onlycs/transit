use std::sync::Arc;

use futures::{AsyncRead, lock::Mutex};
use tracing::{debug, trace, warn};
use transit_core::{
    cancel::CancellationToken,
    frame::{self, *},
    send::ArchSend,
};

use crate::connection::Registry;

pub(super) async fn pframe_decode_thread<R: AsyncRead + Unpin + ArchSend>(
    mut reader: R,
    tx: Arc<Mutex<Registry>>,
    notify: CancellationToken,
) {
    debug!("Frame decode thread started");

    let job = async {
        loop {
            let frame = match frame::next(&mut reader).await {
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

            if frame.len() < MSGID_LEN {
                warn!("Frame less than minimum size, ignoring");
                continue;
            }

            let mut tx = tx.lock().await;
            let mut msgid: MessageId = frame[..MSGID_LEN].try_into().unwrap();
            let res = match msgid[0] & NOT_FOUND_BIT {
                0 => Some(frame[MSGID_LEN..].to_vec()),
                _ => {
                    msgid[0] &= !NOT_FOUND_BIT;
                    debug!(msgid = hex::encode(msgid), "Route not found");
                    None
                }
            };

            let Some(tx) = tx.remove(&msgid) else {
                warn!(msgid = hex::encode(msgid), "Unknown message id, ignoring");
                continue;
            };

            trace!(msgid = hex::encode(msgid), "Received response");

            let Ok(_) = tx.send(res) else {
                warn!(msgid = hex::encode(msgid), "rx dropped, ignoring");
                continue;
            };
        }
    };

    if notify.run_until_cancelled(job).await.is_none() {
        warn!("Frame decode thread cancelled");
    }
}
