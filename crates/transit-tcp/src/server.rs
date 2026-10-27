use std::{io, net::SocketAddr};

use async_net::{TcpListener, TcpStream};
use transit_core::send::ArchSend;
use transit_rt::Runtime;
use transit_server::transport::ServerTransport;

use crate::TransitTcp;

impl ServerTransport for TransitTcp {
    type Error = io::Error;
    type IO = TcpStream;
    type ListenOptions = SocketAddr;
    type Listener = TcpListener;

    fn listen<Rt: Runtime>(
        opts: Self::ListenOptions,
    ) -> impl Future<Output = Result<Self::Listener, Self::Error>> + ArchSend {
        TcpListener::bind(opts)
    }

    #[allow(clippy::manual_async_fn)]
    fn accept<Rt: Runtime>(
        listener: &mut Self::Listener,
    ) -> impl Future<Output = Result<Self::IO, Self::Error>> + ArchSend {
        async move {
            let (stream, _) = listener.accept().await?;
            Ok(stream)
        }
    }
}
