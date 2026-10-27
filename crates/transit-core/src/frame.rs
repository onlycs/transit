use std::{io, mem};

use futures::{AsyncRead, AsyncReadExt};
use snafu::{Location, ResultExt, Snafu};
use tracing::trace;

use crate::send::ArchSend;

pub const NOT_FOUND_BIT: u8 = 0x80;
pub const MAX_FRAME_LEN: usize = 5 * 1024 * 1024; // 5 MiB is more than enough
pub const MSGID_LEN: usize = 16;

pub type FrameLen = u32;
pub type RouteId = u64;
pub type MessageId = [u8; MSGID_LEN];

// useful:
// response (p): {frame len}{message id}{data bytes}
// request (q): {frame len}{message id}{route id}{data bytes}

#[derive(Snafu, Debug)]
#[snafu(display("frame too long (refusing to allocate {:.2}MiB)", *bytes as f64 / 1000.0 / 1000.0))]
pub struct FrameTooLong {
    bytes: usize,
    #[snafu(implicit)]
    #[cfg_attr(feature = "nightly", snafu(provide))]
    location: Location,
}

#[derive(Snafu, Debug)]
pub enum FrameIOError {
    #[snafu(display("{source}"), context(false))]
    TooLong { source: FrameTooLong },

    #[snafu(display("failed to read"))]
    Read {
        source: io::Error,
        #[snafu(implicit)]
        #[cfg_attr(feature = "nightly", snafu(provide))]
        location: Location,
    },
}

pub async fn next<R: AsyncRead + Unpin + ArchSend>(
    reader: &mut R,
) -> Result<Vec<u8>, FrameIOError> {
    let mut len_buf = [0u8; size_of::<FrameLen>()];
    reader.read_exact(&mut len_buf).await.context(ReadSnafu)?;

    let len = FrameLen::from_le_bytes(len_buf) as usize;
    snafu::ensure!(len <= MAX_FRAME_LEN, FrameTooLongSnafu { bytes: len });

    trace!(len, "Reading frame");
    let mut msg_buf = vec![0u8; len];
    reader.read_exact(&mut msg_buf).await.context(ReadSnafu)?;

    Ok(msg_buf)
}

pub fn qframe_encode(
    msgid: MessageId,
    route: RouteId,
    data: Vec<u8>,
) -> Result<Vec<u8>, FrameTooLong> {
    let len = msgid.len() + mem::size_of_val(&route) + data.len();
    snafu::ensure!(len <= MAX_FRAME_LEN, FrameTooLongSnafu { bytes: len });

    trace!(len, msgid = hex::encode(msgid), "Encoding request frame");
    let len_bytes = (len as FrameLen).to_le_bytes();
    let route_bytes = route.to_le_bytes();
    let mut buf = Vec::with_capacity(len + len_bytes.len());

    buf.extend(len_bytes);
    buf.extend(&msgid);
    buf.extend(&route_bytes);
    buf.extend(&data);

    Ok(buf)
}

pub fn pframe_encode(msgid: &MessageId, data: &[u8]) -> Result<Vec<u8>, FrameTooLong> {
    let len = msgid.len() + data.len();
    snafu::ensure!(len <= MAX_FRAME_LEN, FrameTooLongSnafu { bytes: len });

    trace!(len, msgid = hex::encode(msgid), "Encoding response frame");
    let len_bytes = (len as FrameLen).to_le_bytes();
    let mut buf = Vec::with_capacity(len + len_bytes.len());

    buf.extend(len_bytes);
    buf.extend(msgid);
    buf.extend(data);

    Ok(buf)
}

pub fn gen_msgid() -> Result<MessageId, getrandom::Error> {
    let mut msgid = [0u8; MSGID_LEN];

    getrandom::fill(&mut msgid)?;
    msgid[0] &= !NOT_FOUND_BIT;

    Ok(msgid)
}

// pub(super) async fn pframe_deocde_thread<
//     R: Stream<Item = Result<Bytes, io::Error>> + Unpin + ArchSend,
// >(
//     mut reader: R,
//     tx: Arc<Mutex<client::Registry>>,
//     notify: CancellationToken,
// ) {
//     debug!("Frame decode thread started");
//
//     let job = async {
//         loop {
//             let frame = match frame_decode(&mut reader).await {
//                 Ok(frame) => frame,
//                 Err(err) => {
//                     warn!(
//                         "Error reading frame, closing connection. Full
// report:\n{}",
// snafu::Report::from_error(err).to_string()                     );
//                     notify.cancel();
//                     return;
//                 }
//             };
//
//             if frame.len() < MSGID_LEN {
//                 warn!("Frame less than minimum size, ignoring");
//                 continue;
//             }
//
//             let mut tx = tx.lock().await;
//             let mut msgid: MessageId =
// frame[..MSGID_LEN].try_into().unwrap();             let res = match msgid[0]
// & NOT_FOUND_BIT {                 0 => Some(frame[MSGID_LEN..].to_vec()),
//                 _ => {
//                     msgid[0] &= !NOT_FOUND_BIT;
//                     debug!(msgid = hex::encode(msgid), "Route not found");
//                     None
//                 }
//             };
//
//             let Some(tx) = tx.remove(&msgid) else {
//                 warn!(msgid = hex::encode(msgid), "Unknown message id,
// ignoring");                 continue;
//             };
//
//             trace!(msgid = hex::encode(msgid), "Received response");
//
//             let Ok(_) = tx.send(res) else {
//                 warn!(msgid = hex::encode(msgid), "rx dropped, ignoring");
//                 continue;
//             };
//         }
//     };
//
//     if notify.run_until_cancelled(job).await.is_none() {
//         warn!("Frame decode thread cancelled");
//     }
// }

// pub(super) async fn frame_encode_thread<W: AsyncWrite + Unpin + 'static>(
//     mut writer: W,
//     mut rx: UnboundedReceiver<Vec<u8>>,
//     notify: CancellationToken,
// ) {
//     debug!("Frame encode thread started");
//
//     let job = async {
//         loop {
//             let frame = match rx.recv().await.cast_option() {
//                 Some(frame) => frame,
//                 None => {
//                     trace!("rx dropped, closing connection");
//                     notify.cancel();
//                     return;
//                 }
//             };
//
//             if frame.len() < MSGID_LEN {
//                 warn!("Frame less than minimum size, ignoring");
//                 continue;
//             }
//
//             #[cfg(feature = "client")]
//             let timeout = arch::timeout;
//             #[cfg(all(not(feature = "client"), feature = "server"))]
//             let timeout = async |a, b| tokio::time::timeout(a, b).await.ok();
//
//             match timeout(Duration::from_secs(30),
// writer.write_all(&frame)).await {                 Some(Ok(_)) => trace!(len =
// frame.len(), "Wrote frame"),                 Some(Err(err)) => {
//                     warn!(%err, "Error writing frame, closing connection");
//                     notify.cancel();
//                     return;
//                 }
//                 None => {
//                     warn!("Timed out writing frame, closing connection");
//                     notify.cancel();
//                     return;
//                 }
//             }
//
//             if let Err(err) = writer.flush().await {
//                 warn!(%err, "Error flushing writer, closing connection");
//                 notify.cancel();
//                 return;
//             }
//         }
//     };
//
//     if notify.run_until_cancelled(job).await.is_none() {
//         warn!("Frame encode thread cancelled");
//     }
// }
