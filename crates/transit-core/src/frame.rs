use std::{io, mem, time::Duration};

use bilge::arbitrary_int::{u6, u56};
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, Stream, StreamExt};
use snafu::{Location, prelude::*};
use tracing::{debug, trace, warn};
use transit_rt::{Runtime, cancel::CancellationToken, send::ArchSend};

pub const MAX_FRAME_LEN: usize = 5 * 1024 * 1024; // 5 MiB is more than enough
pub const MESSAGE_ID_LEN: usize = mem::size_of::<MessageId>();

pub type FrameLen = u32;
pub type RouteId = u64;

// useful:
// response (p): {frame len}{message id}{data bytes}
// request (q): {frame len}{message id}{route id}{data bytes}

#[derive(Snafu, Debug)]
#[snafu(display("frame too long (refusing to allocate {:.2}MiB)", *bytes as f64 / 1024.0 / 1024.0))]
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

    #[snafu(display("timeout while waiting for frame"))]
    Timeout {
        #[snafu(implicit)]
        #[cfg_attr(feature = "nightly", snafu(provide))]
        location: Location,
    },
}

pub async fn next<Rt: Runtime, R: AsyncRead + Unpin + ArchSend>(
    reader: &mut R,
) -> Result<Vec<u8>, FrameIOError> {
    const INITIAL_BODY_CAP: usize = 64 * 1024;
    const FRAME_TIMEOUT: Duration = Duration::from_secs(30);

    let mut len_buf = [0u8; size_of::<FrameLen>()];
    reader.read_exact(&mut len_buf).await.context(ReadSnafu)?;

    let len = FrameLen::from_le_bytes(len_buf) as usize;
    snafu::ensure!(len <= MAX_FRAME_LEN, FrameTooLongSnafu { bytes: len });

    trace!(len, "Reading frame");
    let mut msg_buf = Vec::with_capacity(len.min(INITIAL_BODY_CAP));
    transit_rt::timeout::<Rt, _>(
        FRAME_TIMEOUT,
        reader.take(len as u64).read_to_end(&mut msg_buf),
    )
    .await
    .context(TimeoutSnafu)?
    .context(ReadSnafu)?;

    Ok(msg_buf)
}

pub fn qframe_encode(
    msgid: MessageId,
    route: RouteId,
    data: impl AsRef<[u8]>,
) -> Result<Vec<u8>, FrameTooLong> {
    let data = data.as_ref();
    let len = msgid.len() + mem::size_of_val(&route) + data.len();
    snafu::ensure!(len <= MAX_FRAME_LEN, FrameTooLongSnafu { bytes: len });

    trace!(
        len,
        msgid = hex::encode(msgid.to_bytes()),
        "Encoding request frame"
    );
    let len_bytes = (len as FrameLen).to_le_bytes();
    let route_bytes = route.to_le_bytes();
    let mut buf = Vec::with_capacity(len + len_bytes.len());

    buf.extend(len_bytes);
    buf.extend(msgid.to_bytes());
    buf.extend(&route_bytes);
    buf.extend(data);

    Ok(buf)
}

pub fn pframe_encode(msgid: &MessageId, data: &[u8]) -> Result<Vec<u8>, FrameTooLong> {
    let len = msgid.len() + data.len();
    snafu::ensure!(len <= MAX_FRAME_LEN, FrameTooLongSnafu { bytes: len });

    trace!(
        len,
        msgid = hex::encode(msgid.to_bytes()),
        "Encoding response frame"
    );
    let len_bytes = (len as FrameLen).to_le_bytes();
    let mut buf = Vec::with_capacity(len + len_bytes.len());

    buf.extend(len_bytes);
    buf.extend(msgid.to_bytes());
    buf.extend(data);

    Ok(buf)
}

#[bilge::bitsize(64)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, bilge::FromBits)]
pub struct MessageId {
    pub unknown_route: bool,
    unstable_hashes: bool,
    reserved: u6,
    rest: u56,
}

impl MessageId {
    pub fn generate() -> Result<MessageId, getrandom::Error> {
        let mut bytes = [0u8; MESSAGE_ID_LEN];
        getrandom::fill(&mut bytes[1..])?;

        Ok(Self::new(
            false,
            cfg!(feature = "unstable"),
            u56::extract_u64(u64::from_le_bytes(bytes), 0),
        ))
    }

    pub const fn to_bytes(self) -> [u8; MESSAGE_ID_LEN] {
        self.value.to_le_bytes()
    }

    pub fn has_equivalent_stability(&self) -> bool {
        self.unstable_hashes() == cfg!(feature = "unstable")
    }

    pub fn set_stability(&mut self) {
        self.set_unstable_hashes(cfg!(feature = "unstable"));
    }

    pub fn is_stable(&self) -> bool {
        !self.unstable_hashes()
    }

    #[allow(clippy::len_without_is_empty)]
    pub const fn len(self) -> usize {
        mem::size_of::<Self>()
    }
}

impl<'a> TryFrom<&'a [u8]> for MessageId {
    type Error = <[u8; MESSAGE_ID_LEN] as TryFrom<&'a [u8]>>::Error;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        let bytes = <[u8; MESSAGE_ID_LEN] as TryFrom<&[u8]>>::try_from(value)?;
        Ok(MessageId::from(u64::from_le_bytes(bytes)))
    }
}

pub async fn encode_thread<
    Rt: Runtime,
    W: AsyncWrite + Unpin + ArchSend + 'static,
    S: Stream<Item = Vec<u8>> + Unpin,
>(
    mut writer: W,
    mut rx: S,
    notify: CancellationToken,
) {
    debug!("Frame encode thread started");

    let job = async {
        loop {
            let frame = match rx.next().await {
                Some(frame) => frame,
                None => {
                    trace!("rx dropped, closing connection");
                    notify.cancel();
                    return;
                }
            };

            if frame.len() < MESSAGE_ID_LEN + mem::size_of::<FrameLen>() {
                warn!("Frame less than minimum size, ignoring");
                continue;
            }

            match transit_rt::timeout::<Rt, _>(Duration::from_secs(30), writer.write_all(&frame))
                .await
            {
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
