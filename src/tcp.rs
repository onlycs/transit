use std::{io::Cursor, sync::Arc, time::Duration};

use rustls::{ClientConfig, RootCertStore, pki_types::ServerName};
use rustls_platform_verifier::BuilderVerifierExt;
use snafu::{Location, prelude::*};
use tokio::{
    io::{ReadHalf, WriteHalf},
    net::{
        TcpStream,
        tcp::{OwnedReadHalf, OwnedWriteHalf},
    },
    time,
};
use tokio_rustls::{TlsConnector, TlsStream};
use tokio_util::either::Either;
use transit_macros::core_error;

#[core_error]
pub enum ConnectError {
    #[snafu(display("Invalid certificate"))]
    RootCertParse {
        source: std::io::Error,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Invalid certificate"))]
    RootCertAdd {
        source: rustls::Error,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Platform verifier error"))]
    PlatformVerifier {
        source: rustls::Error,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Invalid DNS name: {addr}"))]
    DNSName {
        source: rustls::pki_types::InvalidDnsNameError,
        addr: String,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Could not connect to TCP stream at {addr}:{port}"))]
    TcpStreamConnect {
        source: std::io::Error,
        addr: String,
        port: u16,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Could not connect to rustls stream at {addr}:{port}"))]
    RustlsConnect {
        source: std::io::Error,
        addr: String,
        port: u16,
        #[snafu(implicit)]
        location: Location,
    },
}

pub type Reader = Either<OwnedReadHalf, ReadHalf<TlsStream<TcpStream>>>;
pub type Writer = Either<OwnedWriteHalf, WriteHalf<TlsStream<TcpStream>>>;

#[derive(Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum Tls {
    Internal,
    Bytes(Vec<u8>),
}

#[derive(Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConnectOptions {
    pub addr: String,
    pub port: u16,
    pub crt: Option<Tls>,
}

pub(super) async fn connect(
    ConnectOptions { addr, port, crt }: &ConnectOptions,
) -> Result<(Reader, Writer), ConnectError> {
    let port = *port;

    let raw = TcpStream::connect(format!("{addr}:{port}"))
        .await
        .context(TcpStreamConnectSnafu { addr, port })?;

    if let Some(tls) = crt {
        let config = match tls {
            Tls::Bytes(crt) => {
                let mut ca = Cursor::new(crt);
                let mut roots = RootCertStore::empty();

                for cert in rustls_pemfile::certs(&mut ca) {
                    roots
                        .add(cert.context(RootCertParseSnafu)?)
                        .context(RootCertAddSnafu)?;
                }

                ClientConfig::builder()
                    .with_root_certificates(roots)
                    .with_no_client_auth()
            }
            Tls::Internal => ClientConfig::builder()
                .with_platform_verifier()
                .context(PlatformVerifierSnafu)?
                .with_no_client_auth(),
        };

        let connector = TlsConnector::from(Arc::new(config));
        let server_name = ServerName::try_from(addr.as_str()).context(DNSNameSnafu { addr })?;

        let tls = connector
            .connect(server_name.to_owned(), raw)
            .await
            .context(RustlsConnectSnafu { addr, port })?
            .into();

        let (read, write) = tokio::io::split(tls);
        Ok((Either::Right(read), Either::Right(write)))
    } else {
        let (read, write) = raw.into_split();
        Ok((Either::Left(read), Either::Left(write)))
    }
}

pub type JoinHandle<T> = tokio::task::JoinHandle<T>;

pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + Sync + 'static,
    F::Output: Send + Sync + 'static,
{
    tokio::spawn(future)
}

pub async fn timeout<F: Future>(duration: Duration, future: F) -> Option<F::Output> {
    time::timeout(duration, future).await.ok()
}
