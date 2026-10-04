use std::{io::Cursor, sync::Arc};

use rustls::{ClientConfig, RootCertStore, pki_types::ServerName};
use rustls_platform_verifier::BuilderVerifierExt;
use snafu::{Location, prelude::*};
use tracing::{debug, info, trace};
use transit_macros::core_error;

use crate::rt::{
    io::{self, Either, ReadHalf, WriteHalf},
    net::TcpStream,
    tls::{TlsConnector, client::TlsStream},
};

#[core_error]
pub enum ConnectError {
    #[snafu(display("Invalid certificate"))]
    RootCertParse {
        source: std::io::Error,
        #[snafu(implicit)]
        #[cfg_attr(feature = "nightly", snafu(provide))]
        location: Location,
    },

    #[snafu(display("Invalid certificate"))]
    RootCertAdd {
        source: rustls::Error,
        #[snafu(implicit)]
        #[cfg_attr(feature = "nightly", snafu(provide))]
        location: Location,
    },

    #[snafu(display("Platform verifier error"))]
    PlatformVerifier {
        source: rustls::Error,
        #[snafu(implicit)]
        #[cfg_attr(feature = "nightly", snafu(provide))]
        location: Location,
    },

    #[snafu(display("Invalid DNS name: {addr}"))]
    DNSName {
        source: rustls::pki_types::InvalidDnsNameError,
        addr: String,
        #[snafu(implicit)]
        #[cfg_attr(feature = "nightly", snafu(provide))]
        location: Location,
    },

    #[snafu(display("Could not connect to TCP stream at {addr}:{port}"))]
    TcpStreamConnect {
        source: std::io::Error,
        addr: String,
        port: u16,
        #[snafu(implicit)]
        #[cfg_attr(feature = "nightly", snafu(provide))]
        location: Location,
    },

    #[snafu(display("Could not connect to rustls stream at {addr}:{port}"))]
    RustlsConnect {
        source: std::io::Error,
        addr: String,
        port: u16,
        #[snafu(implicit)]
        #[cfg_attr(feature = "nightly", snafu(provide))]
        location: Location,
    },
}

pub type Reader = Either<ReadHalf<TlsStream<TcpStream>>, ReadHalf<TcpStream>>;
pub type Writer = Either<WriteHalf<TlsStream<TcpStream>>, WriteHalf<TcpStream>>;

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

    debug!(addr, port, tls = crt.is_some(), "Connecting");

    let raw = TcpStream::connect(format!("{addr}:{port}"))
        .await
        .context(TcpStreamConnectSnafu { addr, port })?;

    if let Some(tls) = crt {
        trace!("TCP connected, starting TLS handshake");

        let config = match tls {
            Tls::Bytes(crt) => {
                let mut ca = Cursor::new(crt);
                let mut roots = RootCertStore::empty();

                for cert in rustls_pemfile::certs(&mut ca) {
                    roots
                        .add(cert.context(RootCertParseSnafu)?)
                        .context(RootCertAddSnafu)?;
                }

                debug!(count = roots.len(), "Using custom root certificates");

                ClientConfig::builder()
                    .with_root_certificates(roots)
                    .with_no_client_auth()
            }
            Tls::Internal => {
                debug!("Using platform certificate verifier");

                ClientConfig::builder()
                    .with_platform_verifier()
                    .context(PlatformVerifierSnafu)?
                    .with_no_client_auth()
            }
        };

        let connector = TlsConnector::from(Arc::new(config));
        let server_name = ServerName::try_from(addr.as_str()).context(DNSNameSnafu { addr })?;

        let tls = connector
            .connect(server_name.to_owned(), raw)
            .await
            .context(RustlsConnectSnafu { addr, port })?;

        info!(addr, port, tls = true, "Connected");
        let (read, write) = io::split(tls);

        Ok((Either::Left(read), Either::Left(write)))
    } else {
        info!(addr, port, tls = false, "Connected");
        let (read, write) = io::split(raw);
        Ok((Either::Right(read), Either::Right(write)))
    }
}

pub use crate::rt::{spawn, time::timeout};
