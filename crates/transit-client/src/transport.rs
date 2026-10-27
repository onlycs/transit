use std::fmt::Debug;

use futures::{AsyncRead, AsyncWrite};
use transit_core::send::ArchSend;
use transit_rt::Runtime;

pub trait ClientTransport: 'static {
    type IO: AsyncWrite + AsyncRead + Unpin + ArchSend + 'static;

    type ConnectOptions: Clone;
    type ConnectError: snafu::AsErrorSource + Debug;

    fn connect<Rt: Runtime>(
        opts: Self::ConnectOptions,
    ) -> impl Future<Output = Result<Self::IO, Self::ConnectError>> + ArchSend;
}
