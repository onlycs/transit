pub mod cancel;
pub mod send;

use std::{pin::pin, time::Duration};

use futures::future::{self, Either};

use crate::send::ArchSend;

pub trait Runtime: 'static {
    fn spawn(fut: impl Future<Output = ()> + ArchSend + 'static);
    fn sleep(dur: Duration) -> impl Future<Output = ()> + ArchSend;
}

#[allow(clippy::manual_async_fn)]
pub fn timeout<C: Runtime, F: Future + ArchSend>(
    dur: Duration,
    fut: F,
) -> impl Future<Output = Option<F::Output>> + ArchSend
where
    F::Output: ArchSend,
{
    async move {
        let (fut, sleep) = (pin!(fut), pin!(C::sleep(dur)));
        match future::select(fut, sleep).await {
            Either::Left((out, _)) => Some(out),
            Either::Right(_) => None,
        }
    }
}
