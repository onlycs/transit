use std::{self, fmt, sync::Arc};

use transit_async_io::RtAsyncIo;
use transit_client::{TransitClient, TransitOptions};
use transit_core::Route;
use transit_tcp::TransitTcp;
use transit_tls::{
    TransitTls,
    client::{Tls, TlsOptions},
};
use uniffi::{Enum, Object, Record};

uniffi::setup_scaffolding!("transit");

#[derive(Enum)]
pub enum TlsKind {
    Internal,
    Bytes(Vec<u8>),
}

#[derive(Record)]
pub struct ConnectTls {
    pub server_name: String,
    pub kind: TlsKind,
}

#[derive(Record)]
pub struct ConnectOptions {
    pub address: String,
    pub port: u16,
    pub tls: Option<ConnectTls>,
}

#[derive(Object)]
pub enum Transit {
    Tls(TransitClient<RtAsyncIo, TransitTls<TransitTcp>>),
    NoTls(TransitClient<RtAsyncIo, TransitTcp>),
}

#[derive(Clone, Debug, Record)]
pub struct TransitError {
    pub display: String,
    pub report: String,
}

impl<T> From<T> for TransitError
where
    T: fmt::Display + std::error::Error,
{
    fn from(value: T) -> Self {
        Self {
            display: format!("{value}"),
            report: snafu::Report::from_error(&value).to_string(),
        }
    }
}

impl Transit {
    #[uniffi::constructor]
    pub async fn connect(opts: ConnectOptions) -> Result<Arc<Self>, TransitError> {
        match opts.tls {
            Some(with) => Ok(Arc::new(Self::Tls(
                TransitClient::<RtAsyncIo, TransitTls<TransitTcp>>::connect(TransitOptions::new(
                    TlsOptions {
                        inner: format!("{}{}", opts.address, opts.port).parse()?,
                        server_name: with.server_name,
                        tls: match with.kind {
                            TlsKind::Bytes(b) => Tls::Bytes(b),
                            TlsKind::Internal => Tls::Internal,
                        },
                    },
                ))
                .await?,
            ))),
            None => Ok(Arc::new(Self::NoTls(
                TransitClient::connect(TransitOptions::new(
                    format!("{}{}", opts.address, opts.port).parse()?,
                ))
                .await?,
            ))),
        }
    }

    pub async fn route<R: Route>(&self, q: R::Request) -> Result<R::Response, TransitError> {
        match self {
            Self::Tls(tls) => tls.route::<R>(q).await.map_err(TransitError::from),
            Self::NoTls(notls) => notls.route::<R>(q).await.map_err(TransitError::from),
        }
    }
}
