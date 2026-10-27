use std::{io, net::SocketAddr};

use async_net::TcpStream;
use transit_client::transport::ClientTransport;

use crate::TransitTcp;

impl ClientTransport for TransitTcp {
    type ConnectError = io::Error;
    type ConnectOptions = SocketAddr;
    type IO = TcpStream;

    fn connect<Rt: transit_rt::Runtime>(
        opts: Self::ConnectOptions,
    ) -> impl Future<Output = Result<Self::IO, Self::ConnectError>> + transit_core::send::ArchSend
    {
        TcpStream::connect(opts)
    }
}
