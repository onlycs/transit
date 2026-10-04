use std::{
    collections::HashMap, io::Cursor, mem, panic::AssertUnwindSafe, sync::Arc, time::Duration,
};

use futures_util::{FutureExt, future::BoxFuture};
use rustls::server::{VerifierBuilderError, WebPkiClientVerifier};
use snafu::{Location, ResultExt, Snafu};
use tokio_util::sync::CancellationToken;
use tracing::warn;
use wtransport::{Endpoint, error::ConnectionError, tls::WEBTRANSPORT_ALPN};

use crate::{
    InternalError, InternalSnafu, Route,
    frame::{self, MessageId, RouteId, frame_encode_thread},
    route::FromInternal,
    rt::{
        self,
        io::{self, AsyncRead, AsyncWrite, Either},
        mpsc,
        net::{self, TcpListener},
        time,
        tls::TlsAcceptor,
    },
};

#[derive(Snafu, Debug)]
pub enum ListenError {
    #[snafu(display("Failed to bind to {addr}:{port}"))]
    Bind {
        source: io::Error,
        addr: String,
        port: u16,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Failed to parse certificate chain"))]
    CertChain {
        source: io::Error,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Empty certificate chain"))]
    CertChainEmpty {
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Failed to parse private key"))]
    PrivateKey {
        source: io::Error,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Missing private key"))]
    MissingPrivateKey {
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Failed to parse client CA"))]
    ClientCa {
        source: io::Error,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Failed to add client CA"))]
    ClientCaAdd {
        source: rustls::Error,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Failed to build client verifier"))]
    ClientVerifier {
        source: VerifierBuilderError,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Failed to configure server TLS"))]
    ServerConfig {
        source: rustls::Error,
        #[snafu(implicit)]
        location: Location,
    },
}

#[derive(Snafu, Debug)]
pub enum AcceptError {
    #[snafu(display("TLS handshake failure"))]
    Tls {
        source: io::Error,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("WebTransport session failure"))]
    Session {
        source: ConnectionError,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Failed to accept WebTransport stream"))]
    Stream {
        source: ConnectionError,
        #[snafu(implicit)]
        location: Location,
    },
}

#[derive(Snafu, Debug)]
pub enum RoundTripError {
    #[snafu(display("Frame  error"))]
    Frame {
        source: frame::FrameError,
        #[snafu(implicit)]
        location: Location,
    },
}

#[derive(Clone)]
pub struct ServerTls {
    pub cert_chain: Vec<u8>,
    pub private_key: Vec<u8>,
    pub client_ca: Option<Vec<u8>>,
}

#[derive(Clone)]
pub struct TcpListenOptions {
    pub addr: String,
    pub port: u16,
    pub tls: Option<ServerTls>,
}

#[derive(Clone)]
pub struct XwtListenOptions {
    pub addr: String,
    pub port: u16,
    pub path: String,
    pub tls: ServerTls,
}

#[derive(Clone, Copy)]
struct FnErased(*const ());

// SAFETY: FnErased should be a function pointer without a type
// function pointers are always safe to send and sync
unsafe impl Send for FnErased {}
unsafe impl Sync for FnErased {}

struct StateErased(*const ());

// SAFETY: `S` is bounded by Send+Sync inside the struct
unsafe impl Send for StateErased {}
unsafe impl Sync for StateErased {}

type FnRunnerResponse<'a> = BoxFuture<'a, Result<Vec<u8>, InternalError>>;
type FnRunner = for<'a> fn(&'a [u8], FnErased, StateErased) -> FnRunnerResponse<'a>;
type FnEncodeInternal = fn(InternalError) -> Vec<u8>;

/// I refuse to deal with dyn Fn objects, sue me.
#[derive(Clone, Copy)]
pub struct RouteThunk {
    erased: FnErased,
    internal: FnEncodeInternal,
    runner: FnRunner,
}

#[derive(Default)]
pub struct Router<S: Send + Sync + 'static> {
    state: Arc<S>,
    routes: HashMap<RouteId, RouteThunk>,
}

impl<S: Send + Sync + 'static> Router<S> {
    pub fn new(state: S) -> Router<S> {
        Self {
            state: Arc::new(state),
            routes: HashMap::new(),
        }
    }

    pub fn stateless() -> Router<()> {
        Router::new(())
    }

    pub fn route<R: Route, F: Future<Output = R::Response> + Send + 'static>(
        mut self,
        handler: fn(R::Request, Arc<S>) -> F,
    ) -> Self {
        self.routes.insert(
            R::ID,
            RouteThunk {
                runner: |bytes, FnErased(erased), StateErased(state)| {
                    let handler: fn(R::Request, Arc<S>) -> F = unsafe { mem::transmute(erased) };
                    let state: Arc<S> = unsafe { Arc::from_raw(state as *const S) };

                    Box::pin(async move {
                        let req: R::Request = bitcode::decode(bytes).context(InternalSnafu)?;
                        let res = handler(req, state).await;
                        Ok(bitcode::encode(&res))
                    })
                },
                internal: |error| {
                    bitcode::encode(&<R::Response as FromInternal>::from_internal(error))
                },
                erased: FnErased(handler as *const ()),
            },
        );

        self
    }

    pub fn route_stateless<R: Route, F: Future<Output = R::Response> + Send + 'static>(
        mut self,
        handler: fn(R::Request) -> F,
    ) -> Self {
        self.routes.insert(
            R::ID,
            RouteThunk {
                runner: |bytes, FnErased(erased), _| {
                    let handler: fn(R::Request) -> F = unsafe { mem::transmute(erased) };

                    Box::pin(async move {
                        let req: R::Request = bitcode::decode(bytes).context(InternalSnafu)?;
                        let res = handler(req).await;
                        Ok(bitcode::encode(&res))
                    })
                },
                internal: |error| {
                    bitcode::encode(&<R::Response as FromInternal>::from_internal(error))
                },
                erased: FnErased(handler as *const ()),
            },
        );

        self
    }

    pub fn build(self) -> Arc<Self> {
        Arc::new(self)
    }

    /// No data if and only if the route is not found.
    pub async fn run<'a>(&'a self, route_id: RouteId, data: &'a [u8]) -> Option<Vec<u8>> {
        let RouteThunk {
            erased,
            internal: internal_ser,
            runner,
        } = *self.routes.get(&route_id)?;

        let state = Arc::into_raw(Arc::clone(&self.state));
        let state = StateErased(state as *const ());

        let res = AssertUnwindSafe(runner(data, erased, state))
            .catch_unwind()
            .await;

        let bytes = match res {
            Ok(Ok(bytes)) if bytes.len() + mem::size_of::<MessageId>() <= frame::MAX_FRAME_LEN => {
                bytes
            }
            Ok(Ok(_)) => internal_ser(InternalError {
                message: "response too large".to_string(),
            }),
            Ok(Err(internal)) => internal_ser(internal),
            Err(_) => internal_ser(InternalError {
                message: "server panicked while responding".to_string(),
            }),
        };

        Some(bytes)
    }
}

fn server_tls_config(tls: &ServerTls) -> Result<rustls::ServerConfig, ListenError> {
    let certs = rustls_pemfile::certs(&mut Cursor::new(&tls.cert_chain))
        .collect::<Result<Vec<_>, _>>()
        .context(CertChainSnafu)?;

    snafu::ensure!(!certs.is_empty(), CertChainEmptySnafu);

    let key = rustls_pemfile::private_key(&mut Cursor::new(&tls.private_key))
        .context(PrivateKeySnafu)?
        .ok_or_else(|| MissingPrivateKeySnafu.build())?;

    let builder = rustls::ServerConfig::builder();
    let builder = match &tls.client_ca {
        Some(ca) => {
            let mut roots = rustls::RootCertStore::empty();
            for cert in rustls_pemfile::certs(&mut Cursor::new(ca)) {
                roots
                    .add(cert.context(ClientCaSnafu)?)
                    .context(ClientCaAddSnafu)?;
            }

            let verifier = WebPkiClientVerifier::builder(Arc::new(roots))
                .build()
                .context(ClientVerifierSnafu)?;

            builder.with_client_cert_verifier(verifier)
        }
        None => builder.with_no_client_auth(),
    };

    let config = builder
        .with_single_cert(certs, key)
        .context(ServerConfigSnafu)?;

    Ok(config)
}

async fn serve<R, W, S>(mut read: R, write: W, router: Arc<Router<S>>)
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
    S: Send + Sync + 'static,
{
    let cancel = CancellationToken::new();
    let (tx, rx) = mpsc::unbounded();
    rt::spawn(frame_encode_thread(write, rx, cancel.clone()));

    loop {
        // {message id}{route id}{data bytes}
        let fr = match frame::frame_decode(&mut read).await {
            Ok(fr) => fr,
            Err(e) => {
                warn!(
                    "Failed to decode frame, closing connection. Full report:\n{}",
                    snafu::Report::from_error(e).to_string()
                );
                cancel.cancel();
                break;
            }
        };

        let router = Arc::clone(&router);
        let cancel = cancel.clone();
        let tx = tx.clone();

        let routeid_end = frame::MSGID_LEN + size_of::<RouteId>();
        if fr.len() < routeid_end {
            warn!("Request frame too short, closing connection");
            cancel.cancel();
            break;
        }

        rt::spawn(async move {
            let msgid = &fr[..frame::MSGID_LEN].try_into().unwrap();
            let route_id = &fr[frame::MSGID_LEN..routeid_end].try_into().unwrap();
            let data = &fr[routeid_end..];

            let mut msgid: MessageId = *msgid;

            let route_id = RouteId::from_le_bytes(*route_id);
            let result = match router.run(route_id, data).await {
                Some(result) => result,
                None => {
                    msgid[0] |= frame::NOT_FOUND_BIT;
                    vec![]
                }
            };

            let buf = match frame::pframe_encode(&msgid, result.as_slice()) {
                Ok(buf) => buf,
                Err(e) => {
                    warn!(
                        "Failed to encode frame. Full report:\n{}",
                        snafu::Report::from_error(&e).to_string()
                    );

                    return Err(e).context(FrameSnafu)?;
                }
            };

            let Ok(_) = tx.clone().send(buf) else {
                warn!("Rx was dropped, closing connection.");
                cancel.cancel();
                return Ok(());
            };

            Ok::<_, RoundTripError>(())
        });
    }
}

pub async fn listen_tcp_tls<S>(
    TcpListenOptions { addr, port, tls }: TcpListenOptions,
    router: Arc<Router<S>>,
) -> Result<(), ListenError>
where
    S: Send + Sync + 'static,
{
    let listener = TcpListener::bind((addr.as_str(), port))
        .await
        .context(BindSnafu { addr, port })?;

    let acceptor = match tls {
        Some(tls) => Some(TlsAcceptor::from(Arc::new(server_tls_config(&tls)?))),
        None => None,
    };

    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(err) => {
                warn!("Failed to accept connection: {}", err);
                time::sleep(Duration::from_millis(500)).await;
                continue;
            }
        };

        let router = Arc::clone(&router);
        let acceptor = acceptor.clone();

        rt::spawn(async move {
            let (read, write) = match acceptor {
                Some(acceptor) => {
                    let tls = time::timeout(Duration::from_secs(10), acceptor.accept(stream))
                        .await
                        .ok_or_else(|| io::Error::from(io::ErrorKind::TimedOut))
                        .flatten()
                        .context(TlsSnafu)?;

                    let (read, write) = io::split(tls);
                    (Either::Right(read), Either::Right(write))
                }
                None => {
                    let (read, write) = io::split(stream);
                    (Either::Left(read), Either::Left(write))
                }
            };

            serve(read, write, router).await;

            Ok::<_, AcceptError>(())
        });
    }
}

pub async fn listen_xwt<S>(
    XwtListenOptions {
        addr,
        port,
        path,
        tls,
    }: XwtListenOptions,
    router: Arc<Router<S>>,
) -> Result<(), ListenError>
where
    S: Send + Sync + 'static,
{
    let bind = net::lookup_host((addr.as_str(), port))
        .await
        .and_then(|mut addrs| {
            addrs
                .next()
                .ok_or_else(|| io::Error::from(io::ErrorKind::AddrNotAvailable))
        })
        .context(BindSnafu {
            addr: addr.as_str(),
            port,
        })?;

    let mut tls = server_tls_config(&tls)?;
    tls.alpn_protocols = vec![WEBTRANSPORT_ALPN.to_vec()];

    let config = wtransport::ServerConfig::builder()
        .with_bind_address(bind)
        .with_custom_tls(tls)
        .build();

    let endpoint = Endpoint::server(config).context(BindSnafu { addr, port })?;
    let path: Arc<str> = format!("/{}", path.trim_start_matches('/')).into();

    loop {
        let incoming = endpoint.accept().await;
        let router = Arc::clone(&router);
        let path = Arc::clone(&path);

        rt::spawn(async move {
            let request = incoming.await.context(SessionSnafu)?;
            if request.path() != &*path {
                request.not_found().await;
                return Ok(());
            }

            let connection = request.accept().await.context(SessionSnafu)?;
            let (write, read) = connection.accept_bi().await.context(StreamSnafu)?;

            serve(read, write, router).await;

            Ok::<_, AcceptError>(())
        });
    }
}
