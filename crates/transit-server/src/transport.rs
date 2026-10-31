use std::fmt::Debug;

use futures::{AsyncRead, AsyncWrite};
use transit_rt::{Runtime, send::ArchSend};

pub trait ServerTransport: 'static {
    type IO: AsyncRead + AsyncWrite + Unpin + ArchSend + 'static;
    type Error: snafu::AsErrorSource + std::error::Error + Debug + 'static;

    type Pending;
    type Listener;
    type ListenOptions;

    fn listen<Rt: Runtime>(
        opts: Self::ListenOptions,
    ) -> impl Future<Output = Result<Self::Listener, Self::Error>> + ArchSend;

    fn accept<Rt: Runtime>(
        listener: &mut Self::Listener,
    ) -> impl Future<Output = Result<Self::Pending, Self::Error>> + ArchSend;

    fn handshake<Rt: Runtime>(
        pending: Self::Pending,
    ) -> impl Future<Output = Result<Self::IO, Self::Error>> + ArchSend;
}
