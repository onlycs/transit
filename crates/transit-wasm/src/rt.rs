use std::time::Duration;

use transit_core::send::ArchSend;
use transit_rt::Runtime;

pub struct RtWasm;

impl Runtime for RtWasm {
    fn spawn(fut: impl Future<Output = ()> + ArchSend + 'static) {
        wasm_bindgen_futures::spawn_local(fut);
    }

    fn sleep(dur: Duration) -> impl Future<Output = ()> + ArchSend {
        gloo_timers::future::sleep(dur)
    }
}
