use std::fmt::Debug;

use futures::{AsyncRead, AsyncWrite};
use transit_core::send::ArchSend;
use transit_rt::Runtime;

pub trait ServerTransport: 'static {
    type IO: AsyncRead + AsyncWrite + Unpin + ArchSend + 'static;
    type Error: snafu::AsErrorSource + Debug + 'static;

    type Listener;
    type ListenOptions;

    fn listen<Rt: Runtime>(
        opts: Self::ListenOptions,
    ) -> impl Future<Output = Result<Self::Listener, Self::Error>> + ArchSend;

    fn accept<Rt: Runtime>(
        listener: &mut Self::Listener,
    ) -> impl Future<Output = Result<Self::IO, Self::Error>> + ArchSend;
}
