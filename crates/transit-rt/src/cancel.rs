use std::{
    pin::pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use event_listener::Event;
use futures::future::{self, Either};

#[derive(Clone, Default)]
pub struct CancellationToken(Arc<Inner>);

#[derive(Default)]
struct Inner {
    cancelled: AtomicBool,
    event: Event,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        if !self.0.cancelled.swap(true, Ordering::AcqRel) {
            self.0.event.notify(usize::MAX);
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }

    pub async fn cancelled(&self) {
        loop {
            if self.is_cancelled() {
                return;
            }

            self.0.event.listen().await;
        }
    }

    pub async fn run_until_cancelled<F: Future>(&self, fut: F) -> Option<F::Output> {
        let (fut, cancel) = (pin!(fut), pin!(self.cancelled()));
        match future::select(fut, cancel).await {
            Either::Left((out, _)) => Some(out),
            Either::Right(_) => None,
        }
    }
}
