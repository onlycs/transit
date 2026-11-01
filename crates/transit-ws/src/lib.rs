#![cfg_attr(feature = "nightly", feature(error_generic_member_access))]

use std::{
    fmt::Debug,
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use async_tungstenite::{
    ByteReader, ByteWriter, WebSocketReceiver, WebSocketSender, WebSocketStream,
    accept_hdr_async_with_config,
    tungstenite::{
        self, Message,
        handshake::server::{ErrorResponse, Request},
        http::StatusCode,
        protocol::WebSocketConfig,
    },
};
use futures::{AsyncRead, AsyncWrite, Stream, io::BufWriter, ready};
use snafu::prelude::*;
use transit_rt::{Runtime, send::ArchSend};
use transit_server::transport::ServerTransport;

pub struct BinaryOnly<T>(WebSocketReceiver<T>);

impl<T: AsyncRead + AsyncWrite + Unpin> Stream for BinaryOnly<T> {
    type Item = Result<Message, tungstenite::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            return match ready!(Pin::new(&mut self.0).poll_next(cx)) {
                Some(Ok(m @ Message::Binary(_))) => Poll::Ready(Some(Ok(m))),
                Some(Ok(Message::Close(_))) | None => Poll::Ready(None),
                Some(Ok(_)) => continue,
                Some(other) => Poll::Ready(Some(other)),
            };
        }
    }
}

pub struct WsStream<T> {
    read: ByteReader<BinaryOnly<T>>,
    write: BufWriter<ByteWriter<WebSocketSender<T>>>,
}

impl<T: AsyncRead + AsyncWrite + Unpin> WsStream<T> {
    pub fn new(ws: WebSocketStream<T>) -> Self {
        let (tx, rx) = ws.split();
        Self {
            read: ByteReader::new(BinaryOnly(rx)),
            write: BufWriter::new(ByteWriter::new(tx)),
        }
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> AsyncRead for WsStream<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().read).poll_read(cx, buf)
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> AsyncWrite for WsStream<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().write).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().write).poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().write).poll_close(cx)
    }
}

#[derive(Clone)]
pub struct WsListenOptions {
    pub path: String,
    pub config: WebSocketConfig,
}

pub struct WsListener<I: ServerTransport> {
    inner: I::Listener,
    opts: Arc<WsListenOptions>,
}

#[derive(Debug, Snafu)]
pub enum WsError<I: snafu::AsErrorSource> {
    #[snafu(display("connect failed"))]
    Inner { source: I },

    #[snafu(display("websocket handshake failed"))]
    Handshake { source: tungstenite::Error },

    #[snafu(display("websocket handshake timed out"))]
    HandshakeTimeout,
}

pub struct TransitWs<I>(::std::marker::PhantomData<fn(I)>);
impl<I: ServerTransport> ServerTransport for TransitWs<I>
where
    I::Listener: ArchSend,
    I::ListenOptions: ArchSend,
    I::Error: ArchSend,
{
    type Error = WsError<I::Error>;
    type IO = WsStream<I::IO>;
    type ListenOptions = (I::ListenOptions, WsListenOptions);
    type Listener = WsListener<I>;
    type Pending = (I::Pending, Arc<WsListenOptions>);

    async fn listen<Rt: Runtime>(
        (inner, mut opts): Self::ListenOptions,
    ) -> Result<Self::Listener, Self::Error> {
        let inner = I::listen::<Rt>(inner).await.context(InnerSnafu)?;

        opts.path = format!("/{}", opts.path.trim_start_matches('/'));

        Ok(WsListener {
            inner,
            opts: Arc::new(opts),
        })
    }

    async fn accept<Rt: Runtime>(
        listener: &mut Self::Listener,
    ) -> Result<Self::Pending, Self::Error> {
        let pending = I::accept::<Rt>(&mut listener.inner)
            .await
            .context(InnerSnafu)?;

        Ok((pending, Arc::clone(&listener.opts)))
    }

    async fn handshake<Rt: Runtime>((pend, opts): Self::Pending) -> Result<Self::IO, Self::Error> {
        let io: <I as ServerTransport>::IO = I::handshake::<Rt>(pend).await.context(InnerSnafu)?;
        let path = &opts.path;
        let ws = transit_rt::timeout::<Rt, _>(
            Duration::from_secs(10),
            accept_hdr_async_with_config(
                io,
                #[allow(clippy::result_large_err)]
                move |req: &Request, resp| {
                    if req.uri().path() == path {
                        Ok(resp)
                    } else {
                        let mut err = ErrorResponse::new(None);
                        *err.status_mut() = StatusCode::NOT_FOUND;
                        Err(err)
                    }
                },
                Some(opts.config),
            ),
        )
        .await
        .context(HandshakeTimeoutSnafu)?
        .context(HandshakeSnafu)?;

        Ok(WsStream::new(ws))
    }
}
