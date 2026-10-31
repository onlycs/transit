use std::{
    io::{self, Cursor},
    sync::Arc,
    time::Duration,
};

use futures_rustls::{TlsAcceptor, server::TlsStream};
use rustls::server::{VerifierBuilderError, WebPkiClientVerifier};
use snafu::{AsErrorSource, prelude::*};
use tracing::debug;
use transit_macros::core_error;
use transit_rt::{Runtime, send::ArchSend};
use transit_server::transport::ServerTransport;

use crate::TransitTls;

#[core_error]
pub enum TlsError<E: AsErrorSource> {
    #[snafu(display("listen failed"))]
    InnerListen { source: E },

    #[snafu(display("accept failed"))]
    InnerAccept { source: E },

    #[snafu(display("handshake failed"))]
    InnerHandshake { source: E },

    #[snafu(display("tls handshake failure"))]
    Tls { source: io::Error },

    #[snafu(display("failed to parse certificate chain"))]
    CertChain { source: io::Error },

    #[snafu(display("empty certificate chain"))]
    CertChainEmpty,

    #[snafu(display("failed to parse private key"))]
    PrivateKey { source: io::Error },

    #[snafu(display("missing private key"))]
    MissingPrivateKey,

    #[snafu(display("failed to parse client CA"))]
    ClientCa { source: io::Error },

    #[snafu(display("failed to add client CA"))]
    ClientCaAdd { source: rustls::Error },

    #[snafu(display("failed to build client verifier"))]
    ClientVerifier { source: VerifierBuilderError },

    #[snafu(display("failed to configure server TLS"))]
    ServerConfig { source: rustls::Error },
}

#[derive(Clone)]
pub struct TlsOptions<O> {
    pub inner: O,
    pub cert_chain: Vec<u8>,
    pub private_key: Vec<u8>,
    pub client_ca: Option<Vec<u8>>,
}

#[derive(Clone)]
pub struct WithAcceptor<I> {
    pub inner: I,
    pub acceptor: TlsAcceptor,
}

async fn _listen<Rt: Runtime, I: ServerTransport>(
    TlsOptions {
        inner,
        cert_chain,
        private_key,
        client_ca,
    }: TlsOptions<I::ListenOptions>,
) -> Result<WithAcceptor<I::Listener>, TlsError<I::Error>> {
    let certs = rustls_pemfile::certs(&mut Cursor::new(&cert_chain))
        .collect::<Result<Vec<_>, _>>()
        .context(CertChainSnafu)?;

    snafu::ensure!(!certs.is_empty(), CertChainEmptySnafu);

    let key = rustls_pemfile::private_key(&mut Cursor::new(&private_key))
        .context(PrivateKeySnafu)?
        .ok_or_else(|| MissingPrivateKeySnafu.build())?;

    let builder = rustls::ServerConfig::builder();
    let builder = match &client_ca {
        Some(ca) => {
            debug!("Client CA provided, requiring client certificates");
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
        None => {
            debug!("No client CA, skipping client auth");
            builder.with_no_client_auth()
        }
    };

    let config = builder
        .with_single_cert(certs, key)
        .context(ServerConfigSnafu)?;

    let acceptor = TlsAcceptor::from(Arc::new(config));

    Ok(WithAcceptor {
        inner: I::listen::<Rt>(inner).await.context(InnerListenSnafu)?,
        acceptor,
    })
}

async fn _accept<Rt: Runtime, I: ServerTransport>(
    li: &mut WithAcceptor<I::Listener>,
) -> Result<WithAcceptor<I::Pending>, TlsError<I::Error>> {
    let pend = I::accept::<Rt>(&mut li.inner)
        .await
        .context(InnerAcceptSnafu)?;

    Ok(WithAcceptor {
        inner: pend,
        acceptor: li.acceptor.clone(),
    })
}

async fn _handshake<Rt: Runtime, I: ServerTransport>(
    pe: WithAcceptor<I::Pending>,
) -> Result<TlsStream<I::IO>, TlsError<I::Error>> {
    transit_rt::timeout::<Rt, _>(
        Duration::from_secs(10),
        pe.acceptor.accept(
            I::handshake::<Rt>(pe.inner)
                .await
                .context(InnerHandshakeSnafu)?,
        ),
    )
    .await
    .ok_or_else(|| io::Error::from(io::ErrorKind::TimedOut))
    .flatten()
    .context(TlsSnafu)
}

impl<I: ServerTransport> ServerTransport for TransitTls<I>
where
    I::Listener: ArchSend,
    I::ListenOptions: ArchSend,
{
    type Error = TlsError<I::Error>;
    type IO = TlsStream<I::IO>;
    type ListenOptions = TlsOptions<I::ListenOptions>;
    type Listener = WithAcceptor<I::Listener>;
    type Pending = WithAcceptor<I::Pending>;

    fn accept<Rt: Runtime>(
        listener: &mut Self::Listener,
    ) -> impl Future<Output = Result<Self::Pending, Self::Error>> + ArchSend {
        _accept::<Rt, I>(listener)
    }

    fn listen<Rt: Runtime>(
        opts: Self::ListenOptions,
    ) -> impl Future<Output = Result<Self::Listener, Self::Error>> + ArchSend {
        _listen::<Rt, I>(opts)
    }

    fn handshake<Rt: Runtime>(
        pending: Self::Pending,
    ) -> impl Future<Output = Result<Self::IO, Self::Error>> + ArchSend {
        _handshake::<Rt, I>(pending)
    }
}
