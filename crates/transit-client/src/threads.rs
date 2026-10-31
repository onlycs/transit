use std::sync::Arc;

use futures::{AsyncRead, lock::Mutex};
use tracing::{debug, trace, warn};
use transit_core::frame::{self, *};
use transit_macros::core_error;
use transit_rt::{Runtime, cancel::CancellationToken, send::ArchSend};

use crate::connection::Registry;

#[core_error]
pub enum ProtocolError {
    #[snafu(display("unknown route"))]
    UnknownRoute,

    #[snafu(display("stability mismatch"))]
    StabilityMismatch,
}

pub(super) async fn pframe_decode_thread<Rt: Runtime, R: AsyncRead + Unpin + ArchSend>(
    mut reader: R,
    tx: Arc<Mutex<Registry>>,
    notify: CancellationToken,
) {
    debug!("Frame decode thread started");

    let job = async {
        loop {
            let frame = match frame::next::<Rt, _>(&mut reader).await {
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
            let mut msgid = MessageId::try_from(&frame[..MESSAGE_ID_LEN]).unwrap();
            let mut res = Ok(frame[MESSAGE_ID_LEN..].to_vec());

            if !msgid.has_equivalent_stability() {
                warn!(
                    msgid = hex::encode(msgid.to_bytes()),
                    needs_removal = msgid.is_stable(),
                    "The server does not have the same route ID generator as you. Add/remove the `unstable` feature as needed."
                );

                msgid.set_stability();
                res = Err(StabilityMismatchSnafu.build());
            }

            if msgid.unknown_route() {
                warn!(msgid = hex::encode(msgid.to_bytes()), "Route not found");

                msgid.set_unknown_route(false);
                res = Err(UnknownRouteSnafu.build());
            }

            let Some(tx) = tx.remove(&msgid) else {
                warn!(
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
