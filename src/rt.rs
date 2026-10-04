#![allow(unused)]

mod common {
    pub trait CastOption<T> {
        fn cast_option(self) -> Option<T>;
    }

    impl<T> CastOption<T> for Option<T> {
        fn cast_option(self) -> Option<T> {
            self
        }
    }

    impl<T, E> CastOption<T> for Result<T, E> {
        fn cast_option(self) -> Option<T> {
            self.ok()
        }
    }
}

#[cfg(feature = "tokio")]
mod rt_tokio {
    pub mod mpsc {
        use std::fmt;

        use tokio::sync::mpsc::error::SendError as _SendError;
        pub use tokio::sync::mpsc::{
            UnboundedReceiver, UnboundedSender, unbounded_channel as unbounded,
        };

        #[derive(Clone, Copy, Debug)]
        pub struct SendError;

        impl<T> From<_SendError<T>> for SendError {
            fn from(_: _SendError<T>) -> Self {
                SendError
            }
        }

        impl fmt::Display for SendError {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{self:?}")
            }
        }

        impl std::error::Error for SendError {}
    }

    pub mod oneshot {
        pub use tokio::sync::oneshot::{Receiver, Sender, channel, error::RecvError};
    }

    #[cfg(not(target_family = "wasm"))]
    pub mod time {
        pub use tokio::time::*;

        pub async fn timeout<F: Future>(duration: Duration, future: F) -> Option<F::Output> {
            tokio::time::timeout(duration, future).await.ok()
        }
    }

    #[cfg(not(target_family = "wasm"))]
    pub mod tls {
        pub use tokio_rustls::*;
    }

    pub mod io {
        pub use tokio::io::*;
        pub use tokio_util::either::Either;
    }

    pub mod prelude {
        pub use crate::rt::common::CastOption;
    }

    #[cfg(not(target_family = "wasm"))]
    pub use tokio::spawn;
    pub use tokio::{net, sync};
}

#[cfg(feature = "async-io")]
mod rt_async_io {
    pub mod mpsc {
        pub use futures::channel::mpsc::{
            SendError, UnboundedReceiver, UnboundedSender, unbounded,
        };
    }

    pub mod oneshot {
        pub use futures::channel::oneshot::{Canceled as RecvError, Sender, channel};
    }

    pub mod tls {
        pub use futures_rustls::*;
    }

    pub mod io {
        pub use std::io::{Error, ErrorKind};

        pub use futures::future::Either;
        pub use futures_lite::io::*;
    }

    pub mod sync {
        pub use async_lock::Mutex;
    }

    pub mod net {
        pub use async_net::*;

        pub async fn lookup_host<A: AsyncToSocketAddrs>(
            addr: A,
        ) -> Result<<Vec<SocketAddr> as IntoIterator>::IntoIter, super::io::Error> {
            resolve(addr).await.map(|addrs| addrs.into_iter())
        }
    }

    pub mod time {
        use std::time::Duration;

        use async_io::Timer;
        use futures::FutureExt;
        use futures_lite::FutureExt as FuturesLiteExt;

        pub async fn timeout<F: Future>(duration: Duration, future: F) -> Option<F::Output> {
            let run = future.map(Some).or(async move {
                Timer::after(duration).await;
                None
            });

            run.await
        }

        pub async fn sleep(duration: Duration) {
            Timer::after(duration).await;
        }
    }

    pub mod prelude {
        pub use futures::SinkExt;

        pub use crate::rt::common::CastOption;
    }

    fn executor() -> &'static async_executor::Executor<'static> {
        static EXECUTOR: async_executor::Executor = async_executor::Executor::new();
        static DRIVER: std::sync::Once = std::sync::Once::new();

        DRIVER.call_once(|| {
            std::thread::Builder::new()
                .name("transit-executor".into())
                .spawn(|| async_io::block_on(EXECUTOR.run(std::future::pending::<()>())))
                .expect("failed to spawn transit executor thread");
        });

        &EXECUTOR
    }

    pub fn spawn<F>(future: F)
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        executor().spawn(future).detach();
    }
}

#[cfg(feature = "async-io")]
pub use rt_async_io::*;
#[cfg(feature = "tokio")]
pub use rt_tokio::*;
