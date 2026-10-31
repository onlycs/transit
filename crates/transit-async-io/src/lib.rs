use std::time::Duration;

use transit_rt::{Runtime, send::ArchSend};

pub struct RtAsyncIo;

impl RtAsyncIo {
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
}

impl Runtime for RtAsyncIo {
    fn sleep(dur: Duration) -> impl Future<Output = ()> + ArchSend {
        use async_io::Timer;

        async move {
            Timer::after(dur).await;
        }
    }

    fn spawn(fut: impl Future<Output = ()> + ArchSend + 'static) {
        Self::executor().spawn(fut).detach();
    }
}
