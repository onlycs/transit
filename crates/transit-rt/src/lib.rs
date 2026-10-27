use std::{pin::pin, time::Duration};

use futures::{
    AsyncWrite, AsyncWriteExt,
    channel::mpsc::UnboundedReceiver,
    future::{self, Either},
};
use tracing::{debug, trace, warn};
use transit_core::{cancel::CancellationToken, frame::*, send::ArchSend};

pub trait Runtime: 'static {
    fn spawn(fut: impl Future<Output = ()> + ArchSend + 'static);
    fn sleep(dur: Duration) -> impl Future<Output = ()> + ArchSend;
}

#[allow(clippy::manual_async_fn)]
pub fn timeout<C: Runtime, F: Future + ArchSend>(
    dur: Duration,
    fut: F,
) -> impl Future<Output = Option<F::Output>> + ArchSend
where
    F::Output: ArchSend,
{
    async move {
        let (fut, sleep) = (pin!(fut), pin!(C::sleep(dur)));
        match future::select(fut, sleep).await {
            Either::Left((out, _)) => Some(out),
            Either::Right(_) => None,
        }
    }
}

pub async fn frame_encode_thread<Rt: Runtime, W: AsyncWrite + Unpin + ArchSend + 'static>(
    mut writer: W,
    mut rx: UnboundedReceiver<Vec<u8>>,
    notify: CancellationToken,
) {
    debug!("Frame encode thread started");

    let job = async {
        loop {
            let frame = match rx.recv().await {
                Ok(frame) => frame,
                Err(_) => {
                    trace!("rx dropped, closing connection");
                    notify.cancel();
                    return;
                }
            };

            if frame.len() < MSGID_LEN {
                warn!("Frame less than minimum size, ignoring");
                continue;
            }

            match timeout::<Rt, _>(Duration::from_secs(30), writer.write_all(&frame)).await {
                Some(Ok(_)) => trace!(len = frame.len(), "Wrote frame"),
                Some(Err(err)) => {
                    warn!(%err, "Error writing frame, closing connection");
                    notify.cancel();
                    return;
                }
                None => {
                    warn!("Timed out writing frame, closing connection");
                    notify.cancel();
                    return;
                }
            }

            if let Err(err) = writer.flush().await {
                warn!(%err, "Error flushing writer, closing connection");
                notify.cancel();
                return;
            }
        }
    };

    if notify.run_until_cancelled(job).await.is_none() {
        warn!("Frame encode thread cancelled");
    }
}
