use std::{io, pin::pin, sync::Arc, time::Duration};

use async_lock::Semaphore;
use futures::{
    AsyncReadExt, SinkExt,
    channel::mpsc,
    future::{self, Either},
};
use tracing::{debug, trace, warn};
use transit_core::frame::{self, MAX_ALLOWED_FRAME_LEN, MIN_FRAME_LEN, MessageId, RouteId};
use transit_rt::{Runtime, cancel::CancellationToken};

use crate::{router::Router, transport::ServerTransport};

pub mod router;
pub mod transport;

#[derive(Default)]
struct InFlightCounter(Arc<()>);

impl InFlightCounter {
    pub fn acq(&self) -> InFlightCounter {
        InFlightCounter(Arc::clone(&self.0))
    }

    pub fn count(&self) -> usize {
        Arc::strong_count(&self.0)
    }
}

#[derive(Clone, Debug)]
pub struct ServeOptions {
    /// The max size of each frame, in bytes. Defaults to 5MiB (5 * 1024 * 1024)
    pub max_frame_len: usize,

    /// Max requests handled at once, per connection. Requests and responses can
    /// each be up to `max_frame_len`, so worst-case memory per connection
    /// is a small multiple of `max_in_flight * max_frame_len`.
    pub max_in_flight: usize,

    /// Max open connections.
    pub max_connections: usize,

    /// Close a connection after this long with no new requests.
    pub idle_timeout: Duration,

    /// How long a connection keeps reading after a goaway.
    pub goaway_grace: Duration,
}

impl Default for ServeOptions {
    fn default() -> Self {
        Self {
            max_frame_len: 5 * 1024 * 1024,
            max_in_flight: 64,
            max_connections: 512,
            idle_timeout: Duration::from_secs(60),
            goaway_grace: Duration::from_secs(5),
        }
    }
}

pub async fn serve<Rt: Runtime, Tr: ServerTransport, S: Send + Sync>(
    listen_opts: Tr::ListenOptions,
    serve_opts: ServeOptions,
    router: Arc<Router<S>>,
) -> Result<(), Tr::Error> {
    #[cfg(panic = "abort")]
    warn!("panic = \"abort\": a panicking handler will take down the whole server");

    assert!(
        serve_opts.max_frame_len >= MIN_FRAME_LEN,
        "Max frame len is too short, must be at least {MIN_FRAME_LEN}"
    );

    assert!(
        serve_opts.max_frame_len <= MAX_ALLOWED_FRAME_LEN,
        "Max frame len is too long, can be at most {MAX_ALLOWED_FRAME_LEN}"
    );

    let mut listener = Tr::listen::<Rt>(listen_opts).await?;
    let connections = Arc::new(Semaphore::new(serve_opts.max_connections));

    loop {
        let connection = connections.acquire_arc().await;

        let pending: <Tr as ServerTransport>::Pending = match Tr::accept::<Rt>(&mut listener).await
        {
            Ok(it) => it,
            Err(err) => {
                debug!(
                    "Accept failed: {err}. Full report:\n{}",
                    snafu::Report::from_error(&err).to_string()
                );

                Rt::sleep(Duration::from_millis(250)).await;
                continue;
            }
        };

        let router = Arc::clone(&router);

        Rt::spawn(async move {
            let (mut read, write) = match Tr::handshake::<Rt>(pending).await {
                Ok(it) => it.split(),
                Err(err) => {
                    debug!(
                        "Handshake failed: {err}. Full report:\n{}",
                        snafu::Report::from_error(&err).to_string()
                    );

                    return;
                }
            };

            let notify = CancellationToken::new();
            let (tx, rx) = mpsc::channel(serve_opts.max_in_flight);
            let permits = Arc::new(Semaphore::new(serve_opts.max_in_flight));

            let in_flight = InFlightCounter::default();
            let mut draining = false;

            Rt::spawn({
                let fut = frame::encode_thread::<Rt, _, _>(write, rx, notify.clone());

                async move {
                    let _connection = connection;
                    fut.await
                }
            });

            debug!("Serving new connection");
            loop {
                let Some(permit) = notify.run_until_cancelled(permits.acquire_arc()).await else {
                    break;
                };

                let mut fr = pin!(frame::next::<Rt, _>(&mut read, serve_opts.max_frame_len));
                let next = loop {
                    let wait = match draining {
                        true => serve_opts.goaway_grace,
                        false => serve_opts.idle_timeout,
                    };

                    match future::select(fr.as_mut(), pin!(Rt::sleep(wait))).await {
                        Either::Left((next, _)) => break Some(next),
                        Either::Right(_) if in_flight.count() > 1 => {
                            draining = false;
                            continue;
                        }
                        Either::Right(_) if draining => break None,
                        Either::Right(_) => {
                            trace!("Connection idle, sending goaway");
                            draining = true;

                            if tx
                                .clone()
                                .send(
                                    frame::pframe_encode(&MessageId::empty_goaway(), &[])
                                        .expect("Just a message id, shouldn't be too long"),
                                )
                                .await
                                .is_err()
                            {
                                break None;
                            }
                        }
                    }
                };

                let Some(next) = next else {
                    trace!("Connection idle, closing");
                    break;
                };

                let fr = match next {
                    Ok(fr) => fr,
                    Err(frame::FrameIOError::Read { source, .. })
                        if source.kind() == io::ErrorKind::UnexpectedEof =>
                    {
                        trace!("Client disconnected");
                        break;
                    }
                    Err(e) => {
                        warn!(
                            "Failed to decode frame, closing connection. Full report:\n{}",
                            snafu::Report::from_error(e).to_string()
                        );
                        notify.cancel();
                        break;
                    }
                };

                let router: Arc<Router<S>> = Arc::clone(&router);
                let notify = notify.clone();
                let tx = tx.clone();
                let in_flight: InFlightCounter = in_flight.acq();

                let routeid_end = frame::MESSAGE_ID_LEN + size_of::<RouteId>();
                if fr.len() < routeid_end {
                    warn!("Request frame too short, closing connection");
                    notify.cancel();
                    break;
                }

                Rt::spawn(async move {
                    let _permit = permit;
                    let _in_flight = in_flight;

                    let msgid = &fr[..frame::MESSAGE_ID_LEN].try_into().unwrap();
                    let route_id = &fr[frame::MESSAGE_ID_LEN..routeid_end].try_into().unwrap();
                    let data = &fr[routeid_end..];

                    let mut msgid: MessageId = *msgid;
                    trace!(msgid = hex::encode(msgid.to_bytes()), "Received request");

                    let route_id = RouteId::from_le_bytes(*route_id);
                    let result = match msgid.has_equivalent_stability() {
                        true => match router.run(route_id, data, serve_opts.max_frame_len).await {
                            Some(result) => result,
                            None => {
                                msgid.set_unknown_route(true);
                                vec![]
                            }
                        },
                        false => {
                            msgid.set_stability();
                            vec![]
                        }
                    };

                    let buf = frame::pframe_encode(&msgid, result.as_slice())
                        .expect("We already checked for this in the asserts at the beginning of the function");

                    let Ok(_) = tx.clone().send(buf).await else {
                        warn!("Rx was dropped, closing connection.");
                        notify.cancel();
                        return;
                    };
                });
            }
        });
    }
}
