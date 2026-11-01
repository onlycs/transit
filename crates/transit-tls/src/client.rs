use std::{
    io::{self, Cursor},
    sync::Arc,
};

use futures_rustls::{TlsConnector, client::TlsStream};
use rustls::{ClientConfig, RootCertStore, pki_types::ServerName};
use rustls_platform_verifier::BuilderVerifierExt;
use snafu::{AsErrorSource, prelude::*};
use tracing::debug;
use transit_client::transport::ClientTransport;
use transit_macros::core_error;
use transit_rt::{Runtime, send::ArchSend};

use crate::TransitTls;

#[core_error]
pub enum TlsError<E: AsErrorSource> {
    #[snafu(display("invalid certificate"))]
    RootCertParse { source: io::Error },

    #[snafu(display("invalid certificate"))]
    RootCertAdd { source: rustls::Error },

    #[snafu(display("invalid client cert chain"))]
    ClientCertParse { source: io::Error },

    #[snafu(display("invalid client private key"))]
    ClientKeyParse { source: io::Error },

    #[snafu(display("missing private key"))]
    ClientKeyMissing,

    #[snafu(display("invalid client auth"))]
    ClientAuth { source: rustls::Error },

    #[snafu(display("platform verifier error"))]
    PlatformVerifier { source: rustls::Error },

    #[snafu(display("invalid DNS name: {addr}"))]
    DNSName {
        source: rustls::pki_types::InvalidDnsNameError,
        addr: String,
    },

    #[snafu(display("could not connect to rustls stream"))]
    RustlsConnect { source: io::Error },

    #[snafu(display("connection failed"))]
    InnerConnect { source: E },
}

#[derive(Clone)]
pub enum RootCa {
    Internal,
    Bytes(Vec<u8>),
}

#[derive(Clone)]
pub struct ClientAuth {
    pub cert_chain: Vec<u8>,
    pub key_der: Vec<u8>,
}

#[derive(Clone)]
pub struct TlsOptions<O> {
    pub inner: O,
    pub server_name: String,
    pub ca: RootCa,
    pub client: Option<ClientAuth>,
}

async fn _connect<Rt: Runtime, I: ClientTransport>(
    TlsOptions {
        inner,
        server_name,
        ca,
        client,
    }: TlsOptions<I::ConnectOptions>,
) -> Result<TlsStream<I::IO>, TlsError<I::ConnectError>> {
    let conn = I::connect::<Rt>(inner).await.context(InnerConnectSnafu)?;

    debug!("Performing TLS handshake");

    let config = match ca {
        RootCa::Bytes(crt) => {
            let mut ca = Cursor::new(crt);
            let mut roots = RootCertStore::empty();

            for cert in rustls_pemfile::certs(&mut ca) {
                roots
                    .add(cert.context(RootCertParseSnafu)?)
                    .context(RootCertAddSnafu)?;
            }

            debug!(count = roots.len(), "Using custom root certificates");

            ClientConfig::builder().with_root_certificates(roots)
        }
        RootCa::Internal => {
            debug!("Using platform certificate verifier");

            ClientConfig::builder()
                .with_platform_verifier()
                .context(PlatformVerifierSnafu)?
        }
    };

    let config = match client {
        Some(auth) => config
            .with_client_auth_cert(
                rustls_pemfile::certs(&mut Cursor::new(auth.cert_chain))
                    .collect::<Result<Vec<_>, _>>()
                    .context(ClientCertParseSnafu)?,
                rustls_pemfile::private_key(&mut Cursor::new(auth.key_der))
                    .context(ClientKeyParseSnafu)?
                    .context(ClientKeyMissingSnafu)?,
            )
            .context(ClientAuthSnafu)?,
        None => config.with_no_client_auth(),
    };

    let connector = TlsConnector::from(Arc::new(config));
    let server_name = ServerName::try_from(server_name.as_str())
        .map(|it| it.to_owned())
        .context(DNSNameSnafu { addr: server_name })?;

    let tls = connector
        .connect(server_name, conn)
        .await
        .context(RustlsConnectSnafu)?;

    Ok(tls)
}

impl<I: ClientTransport> ClientTransport for TransitTls<I>
where
    I::ConnectOptions: ArchSend,
{
    type ConnectError = TlsError<I::ConnectError>;
    type ConnectOptions = TlsOptions<I::ConnectOptions>;
    type IO = TlsStream<I::IO>;

    fn connect<Rt: Runtime>(
        opts: Self::ConnectOptions,
    ) -> impl Future<Output = Result<Self::IO, Self::ConnectError>> + ArchSend {
        _connect::<Rt, I>(opts)
    }
}
