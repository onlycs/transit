use std::time::Duration;

use transit_rt::{Runtime, send::ArchSend};

pub struct RtTokio;

impl Runtime for RtTokio {
    fn sleep(dur: Duration) -> impl Future<Output = ()> + ArchSend {
        tokio::time::sleep(dur)
    }

    fn spawn(fut: impl Future<Output = ()> + ArchSend + 'static) {
        tokio::spawn(fut);
    }
}
