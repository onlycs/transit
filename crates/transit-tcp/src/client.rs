use std::io;

use async_net::TcpStream;
use transit_client::transport::ClientTransport;
use transit_rt::send::ArchSend;

use crate::TransitTcp;

impl ClientTransport for TransitTcp {
    type ConnectError = io::Error;
    type ConnectOptions = String;
    type IO = TcpStream;

    fn connect<Rt: transit_rt::Runtime>(
        opts: Self::ConnectOptions,
    ) -> impl Future<Output = Result<Self::IO, Self::ConnectError>> + ArchSend {
        TcpStream::connect(opts)
    }
}
