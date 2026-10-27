use std::{
    fmt::Debug,
    io,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use async_net::{TcpListener, TcpStream};
use async_tungstenite::{
    ByteReader, ByteWriter, WebSocketReceiver, WebSocketSender, WebSocketStream, accept_hdr_async,
    tungstenite::{
        self, Message,
        handshake::server::{ErrorResponse, Request},
        http::StatusCode,
    },
};
use futures::{AsyncRead, AsyncWrite, Stream, io::BufWriter, ready};
use snafu::{OptionExt, ResultExt, Snafu};
use transit_rt::Runtime;
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

pub struct WsListenOptions {
    pub addr: SocketAddr,
    pub path: String,
}

pub struct WsListener {
    tcp: TcpListener,
    path: Arc<str>,
}

#[derive(Debug, Snafu)]
pub enum WsError {
    #[snafu(display("failed to bind {addr}"))]
    Bind { addr: SocketAddr, source: io::Error },

    #[snafu(display("failed to accept TCP connection"))]
    Accept { source: io::Error },

    #[snafu(display("websocket handshake failed"))]
    Handshake { source: tungstenite::Error },

    #[snafu(display("websocket handshake timed out"))]
    HandshakeTimeout,
}

pub struct TransitWs;
impl ServerTransport for TransitWs {
    type Error = WsError;
    type IO = WsStream<TcpStream>;
    type ListenOptions = WsListenOptions;
    type Listener = WsListener;

    async fn listen<Rt: Runtime>(opts: WsListenOptions) -> Result<WsListener, WsError> {
        let tcp = TcpListener::bind(opts.addr)
            .await
            .context(BindSnafu { addr: opts.addr })?;
        let path = format!("/{}", opts.path.trim_start_matches('/')).into();
        Ok(WsListener { tcp, path })
    }

    async fn accept<Rt: Runtime>(listener: &mut WsListener) -> Result<Self::IO, WsError> {
        let (stream, _peer) = listener.tcp.accept().await.context(AcceptSnafu)?;
        let _ = stream.set_nodelay(true);

        let ws = transit_rt::timeout::<Rt, _>(
            Duration::from_secs(10),
            accept_hdr_async(
                stream,
                #[allow(clippy::result_large_err)]
                move |req: &Request, resp| {
                    if req.uri().path() == &*listener.path {
                        Ok(resp)
                    } else {
                        let mut err = ErrorResponse::new(None);
                        *err.status_mut() = StatusCode::NOT_FOUND;
                        Err(err)
                    }
                },
            ),
        )
        .await
        .context(HandshakeTimeoutSnafu)?
        .context(HandshakeSnafu)?;

        Ok(WsStream::new(ws))
    }
}
