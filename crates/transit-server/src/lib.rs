use std::{io, sync::Arc};

use futures::{AsyncReadExt, SinkExt, channel::mpsc};
use tracing::{debug, trace, warn};
use transit_core::{
    cancel::CancellationToken,
    frame::{self, MessageId, RouteId},
};
use transit_rt::Runtime;

use crate::{router::Router, transport::ServerTransport};

pub mod router;
pub mod transport;

pub async fn serve<Rt: Runtime, Tr: ServerTransport, S: Send + Sync>(
    options: Tr::ListenOptions,
    router: Arc<Router<S>>,
) -> Result<(), Tr::Error> {
    let mut listener = Tr::listen::<Rt>(options).await?;

    loop {
        let (mut read, write) = Tr::accept::<Rt>(&mut listener).await?.split();
        let router = Arc::clone(&router);

        debug!("Serving new connection");

        let notify = CancellationToken::new();
        let (tx, rx) = mpsc::unbounded();

        Rt::spawn(transit_rt::frame_encode_thread::<Rt, _>(
            write,
            rx,
            notify.clone(),
        ));

        Rt::spawn(async move {
            loop {
                let fr = match frame::next(&mut read).await {
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

                let routeid_end = frame::MSGID_LEN + size_of::<RouteId>();
                if fr.len() < routeid_end {
                    warn!("Request frame too short, closing connection");
                    notify.cancel();
                    break;
                }

                Rt::spawn(async move {
                    let msgid = &fr[..frame::MSGID_LEN].try_into().unwrap();
                    let route_id = &fr[frame::MSGID_LEN..routeid_end].try_into().unwrap();
                    let data = &fr[routeid_end..];

                    let mut msgid: MessageId = *msgid;
                    trace!(msgid = hex::encode(msgid), "Received request");

                    let route_id = RouteId::from_le_bytes(*route_id);
                    let result = match router.run(route_id, data).await {
                        Some(result) => result,
                        None => {
                            msgid[0] |= frame::NOT_FOUND_BIT;
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
