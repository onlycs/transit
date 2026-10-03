use std::io;
#[cfg(any(feature = "client", feature = "server"))]
use std::time::Duration;
#[cfg(feature = "client")]
use std::{mem, sync::Arc};

use snafu::{Location, ResultExt, Snafu};
#[cfg(feature = "client")]
use tokio::sync::Mutex;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::mpsc::UnboundedReceiver,
};
use tokio_util::sync::CancellationToken;
use tracing::warn;

pub const NOT_FOUND_BIT: u8 = 0x80;
pub const MAX_FRAME_LEN: usize = 5 * 1024 * 1024; // 5 MiB is more than enough
pub const MSGID_LEN: usize = 16;

pub type FrameLen = u32;
pub type RouteId = u64;
pub type MessageId = [u8; MSGID_LEN];

#[cfg(feature = "client")]
use crate::{arch, client};

// useful:
// response (p): {frame len}{message id}{data bytes}
// request (q): {frame len}{message id}{route id}{data bytes}

#[derive(Snafu, Debug)]
pub enum FrameError {
    #[snafu(display("Frame too long (refusing to allocate {size:.2}MiB)"))]
    FrameTooLong {
        size: f32,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Failed to generate message id"))]
    MessageId {
        source: getrandom::Error,
        #[snafu(implicit)]
        location: Location,
    },
}

#[derive(Snafu, Debug)]
pub(super) enum FrameIOError {
    #[cfg(any(feature = "client", feature = "server"))]
    #[snafu(display("{source}"))]
    Frame {
        source: FrameError,
        #[snafu(implicit)]
        location: Location,
    },

    #[cfg(any(feature = "client", feature = "server"))]
    #[snafu(display("Failed to read"))]
    Read {
        source: io::Error,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Write timeout"))]
    Write {
        #[snafu(implicit)]
        location: Location,
    },
}

#[cfg(any(feature = "client", feature = "server"))]
pub(super) async fn frame_decode<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<Vec<u8>, FrameIOError> {
    let mut len_buf = [0u8; size_of::<FrameLen>()];
    reader.read_exact(&mut len_buf).await.context(ReadSnafu)?;

    let len = FrameLen::from_le_bytes(len_buf) as usize;
    if len > MAX_FRAME_LEN {
        return Err(FrameTooLongSnafu {
            size: len as f32 / 1024f32 / 1024f32,
        }
        .build())
        .context(FrameSnafu);
    }

    let mut msg_buf = vec![0u8; len];
    reader.read_exact(&mut msg_buf).await.context(ReadSnafu)?;

    Ok(msg_buf)
}

#[cfg(feature = "client")]
pub(super) fn qframe_encode(
    msgid: MessageId,
    route: RouteId,
    data: Vec<u8>,
) -> Result<Vec<u8>, FrameError> {
    let len = msgid.len() + mem::size_of_val(&route) + data.len();
    if len > MAX_FRAME_LEN {
        return Err(FrameTooLongSnafu {
            size: len as f32 / 1024f32 / 1024f32,
        }
        .build());
    }

    let len_bytes = (len as FrameLen).to_le_bytes();
    let route_bytes = route.to_le_bytes();
    let mut buf = Vec::with_capacity(len + len_bytes.len());

    buf.extend(len_bytes);
    buf.extend(&msgid);
    buf.extend(&route_bytes);
    buf.extend(&data);

    Ok(buf)
}

#[cfg(feature = "server")]
pub(super) fn pframe_encode(msgid: &MessageId, data: &[u8]) -> Result<Vec<u8>, FrameError> {
    let len = msgid.len() + data.len();
    let len_bytes = (len as FrameLen).to_le_bytes();

    if len > MAX_FRAME_LEN {
        return Err(FrameTooLongSnafu {
            size: len as f32 / 1024f32 / 1024f32,
        }
        .build());
    }

    let mut buf = Vec::with_capacity(len + len_bytes.len());
    buf.extend(len_bytes);
    buf.extend(msgid);
    buf.extend(data);

    Ok(buf)
}

#[cfg(feature = "client")]
pub(super) fn gen_msgid() -> Result<MessageId, FrameError> {
    let mut msgid = [0u8; MSGID_LEN];

    getrandom::fill(&mut msgid).context(MessageIdSnafu)?;
    msgid[0] &= !NOT_FOUND_BIT;

    Ok(msgid)
}

#[cfg(feature = "client")]
pub(super) async fn pframe_deocde_thread<R: AsyncRead + Unpin + 'static>(
    mut reader: R,
    tx: Arc<Mutex<client::Registry>>,
    notify: CancellationToken,
) {
    let job = async {
        loop {
            let frame = match frame_decode(&mut reader).await {
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
                    None
                }
            };

            let Some(tx) = tx.remove(&msgid) else {
                warn!("Unknown message id {}, ignoring", hex::encode(msgid));
                continue;
            };

            let Ok(_) = tx.send(res) else {
                warn!("rx dropped for message {}, ignoring", hex::encode(msgid));
                continue;
            };
        }
    };

    if notify.run_until_cancelled(job).await.is_none() {
        warn!("Frame decode thread cancelled");
    }
}

#[cfg(any(feature = "client", feature = "server"))]
pub(super) async fn frame_encode_thread<W: AsyncWrite + Unpin + 'static>(
    mut writer: W,
    mut rx: UnboundedReceiver<Vec<u8>>,
    notify: CancellationToken,
) {
    let job = async {
        loop {
            let frame = match rx.recv().await {
                Some(frame) => frame,
                None => {
                    warn!("rx dropped, closing connection");
                    notify.cancel();
                    return;
                }
            };

            if frame.len() < MSGID_LEN {
                warn!("Frame less than minimum size, ignoring");
                continue;
            }

            #[cfg(feature = "client")]
            let timeout = arch::timeout;
            #[cfg(all(not(feature = "client"), feature = "server"))]
            let timeout = async |a, b| tokio::time::timeout(a, b).await.ok();

            match timeout(Duration::from_secs(30), writer.write_all(&frame)).await {
                Some(Ok(_)) => {}
                _ => {
                    warn!("Error writing frame, closing connection");
                    notify.cancel();
                    return;
                }
            }
        }
    };

    if notify.run_until_cancelled(job).await.is_none() {
        warn!("Frame encode thread cancelled");
    }
}
