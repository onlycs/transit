use std::{io, sync::Arc, time::Duration};

use async_lock::Semaphore;
use futures::{AsyncReadExt, SinkExt, channel::mpsc};
use tracing::{debug, trace, warn};
use transit_core::frame::{self, MessageId, RouteId};
use transit_rt::{Runtime, cancel::CancellationToken};

use crate::{router::Router, transport::ServerTransport};

pub mod router;
pub mod transport;

#[derive(Clone, Debug)]
pub struct ServeOptions {
    pub max_in_flight: usize,
}

impl Default for ServeOptions {
    fn default() -> Self {
        Self { max_in_flight: 64 }
    }
}

pub async fn serve<Rt: Runtime, Tr: ServerTransport, S: Send + Sync>(
    listen_opts: Tr::ListenOptions,
    serve_opts: ServeOptions,
    router: Arc<Router<S>>,
) -> Result<(), Tr::Error> {
    let mut listener = Tr::listen::<Rt>(listen_opts).await?;

    loop {
        let pending = match Tr::accept::<Rt>(&mut listener).await {
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

            Rt::spawn(frame::encode_thread::<Rt, _, _>(write, rx, notify.clone()));

            debug!("Serving new connection");
            loop {
                let Some(permit) = notify.run_until_cancelled(permits.acquire_arc()).await else {
                    break;
                };

                let fr = match frame::next::<Rt, _>(&mut read).await {
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

                let router = Arc::clone(&router);
                let notify = notify.clone();
                let tx = tx.clone();

                let routeid_end = frame::MESSAGE_ID_LEN + size_of::<RouteId>();
                if fr.len() < routeid_end {
                    warn!("Request frame too short, closing connection");
                    notify.cancel();
                    break;
                }

                Rt::spawn(async move {
                    let _permit = permit;

                    let msgid = &fr[..frame::MESSAGE_ID_LEN].try_into().unwrap();
                    let route_id = &fr[frame::MESSAGE_ID_LEN..routeid_end].try_into().unwrap();
                    let data = &fr[routeid_end..];

                    let mut msgid: MessageId = *msgid;
                    trace!(msgid = hex::encode(msgid.to_bytes()), "Received request");

                    let route_id = RouteId::from_le_bytes(*route_id);
                    let result = match msgid.has_equivalent_stability() {
                        true => match router.run(route_id, data).await {
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

                    let buf = match frame::pframe_encode(&msgid, result.as_slice()) {
                        Ok(buf) => buf,
                        Err(e) => {
                            warn!(
                                "Failed to encode response frame. Full report:\n{}",
                                snafu::Report::from_error(&e).to_string()
                            );

                            return;
                        }
                    };

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
