use std::{pin::pin, time::Duration};

use gloo_timers::future::TimeoutFuture;
use snafu::{Location, prelude::*};
use strum::EnumDiscriminants;
use tokio::{io::AsyncWrite, sync::oneshot};
use wasm_bindgen::prelude::*;
use xwt_web::{
    Endpoint,
    core::{
        endpoint::{Connect, connect::Connecting},
        session::stream::{OpenBi, OpeningBi},
    },
};

#[derive(Snafu, Debug, EnumDiscriminants)]
#[strum_discriminants(wasm_bindgen)]
#[strum_discriminants(name(ConnectErrorTag))]
pub enum ConnectErrorInner {
    #[snafu(display("Could not start WebTransport connection to {url}"))]
    Connect {
        source: xwt_web::Error,
        url: String,

        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Could not start WebTransport session to {url}"))]
    Session {
        source: xwt_web::Error,
        url: String,

        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Could not open WebTransport stream to {url}"))]
    Stream {
        source: xwt_web::Error,
        url: String,

        #[snafu(implicit)]
        location: Location,
    },
}

#[derive(Debug)]
#[wasm_bindgen]
pub struct ConnectError {
    error: ConnectErrorInner,
    tag: ConnectErrorTag,
}

#[wasm_bindgen]
impl ConnectError {
    #[wasm_bindgen(getter)]
    pub fn tag(&self) -> ConnectErrorTag {
        self.tag
    }

    #[wasm_bindgen(unchecked_return_type = "never")]
    pub fn raise(&self) -> JsValue {
        wasm_bindgen::throw_str(&snafu::Report::from_error(&self.error).to_string());
    }
}

impl ConnectError {
    pub fn inner(&self) -> &ConnectErrorInner {
        &self.error
    }
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for ConnectError {}

#[derive(Clone)]
#[wasm_bindgen(getter_with_clone)]
pub struct ConnectOptions {
    pub addr: String,
    pub path: String,
}

#[wasm_bindgen]
impl ConnectOptions {
    #[wasm_bindgen(constructor)]
    pub fn new(addr: String, path: String) -> Self {
        Self { addr, path }
    }
}

pub struct KeepAlive {
    send: xwt_web::SendStream,
    _keep_alive: xwt_web::Session,
}

impl AsyncWrite for KeepAlive {
    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        AsyncWrite::poll_flush(pin!(&mut self.send), cx)
    }

    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        AsyncWrite::poll_write(pin!(&mut self.send), cx, buf)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        AsyncWrite::poll_shutdown(pin!(&mut self.send), cx)
    }

    fn is_write_vectored(&self) -> bool {
        self.send.is_write_vectored()
    }

    fn poll_write_vectored(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> std::task::Poll<std::io::Result<usize>> {
        AsyncWrite::poll_write_vectored(pin!(&mut self.send), cx, bufs)
    }
}

pub type Reader = xwt_web::RecvStream;
pub type Writer = KeepAlive;

pub(super) async fn connect(
    ConnectOptions { addr, path }: &ConnectOptions,
) -> Result<(Reader, Writer), ConnectError> {
    async fn inner(addr: &String, path: &String) -> Result<(Reader, Writer), ConnectErrorInner> {
        let url = format!("https://{addr}/{path}");

        let endpoint = Endpoint::default();

        let connecting = endpoint
            .connect(&url)
            .await
            .context(ConnectSnafu { url: &url })?;

        let session = connecting
            .wait_connect()
            .await
            .context(SessionSnafu { url: &url })?;

        let opening = session.open_bi().await.context(StreamSnafu { url: &url })?;
        let (send, recv) = opening.wait_bi().await.unwrap_or_else(|e| match e {});

        Ok((
            recv,
            KeepAlive {
                send,
                _keep_alive: session,
            },
        ))
    }

    match inner(addr, path).await {
        Ok(transit) => Ok(transit),

        Err(error) => Err(ConnectError {
            tag: strum::IntoDiscriminant::discriminant(&error),
            error,
        }),
    }
}

pub type JoinHandle<T> = oneshot::Receiver<T>;

pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + 'static,
    F::Output: Send + Sync + 'static,
{
    let (tx, rx) = oneshot::channel();
    wasm_bindgen_futures::spawn_local(async move {
        let _ = tx.send(future.await);
    });

    rx
}

pub async fn timeout<F: Future>(duration: Duration, future: F) -> Option<F::Output> {
    tokio::select! {
        res = future => Some(res),
        _ = TimeoutFuture::new(duration.as_millis() as u32) => None
    }
}
