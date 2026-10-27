use std::{
    io,
    pin::{Pin, pin},
    task::{Context, Poll},
};

use futures::{AsyncRead, AsyncWrite};
use transit_client::transport::ClientTransport;
use transit_core::send::ArchSend;
use transit_rt::Runtime;
use ws_stream_wasm::{WsErr, WsMeta};

pub struct TransitWsBrowser;

pub struct BrowserWsIo {
    io: async_io_stream::IoStream<ws_stream_wasm::WsStreamIo, Vec<u8>>,
    _meta: WsMeta,
}

impl AsyncRead for BrowserWsIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        pin!(&mut self.io).poll_read(cx, buf)
    }
}

impl AsyncWrite for BrowserWsIo {
    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        pin!(&mut self.io).poll_close(cx)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        pin!(&mut self.io).poll_flush(cx)
    }

    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        pin!(&mut self.io).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        pin!(&mut self.io).poll_write_vectored(cx, bufs)
    }
}

impl ClientTransport for TransitWsBrowser {
    type ConnectError = WsErr;
    type ConnectOptions = String;
    type IO = BrowserWsIo;

    #[allow(clippy::manual_async_fn)]
    fn connect<Rt: Runtime>(
        url: String,
    ) -> impl Future<Output = Result<BrowserWsIo, WsErr>> + ArchSend {
        async move {
            let (meta, ws) = WsMeta::connect(url, None).await?;
            Ok(BrowserWsIo {
                io: ws.into_io(),
                _meta: meta,
            })
        }
    }
}
