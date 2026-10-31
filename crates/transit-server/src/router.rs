use std::{collections::HashMap, mem, panic::AssertUnwindSafe, sync::Arc};

use futures::FutureExt;
use snafu::prelude::*;
use tracing::{debug, error, info, trace, warn};
use transit_core::{
    InternalError, InternalSnafu, Route,
    frame::{self, MessageId, RouteId},
    route::{FromInternal, RouteExt},
};
use transit_rt::send::ArchBoxFuture;

type Runner<S> = Box<
    dyn for<'a> Fn(&'a [u8], Arc<S>) -> ArchBoxFuture<'a, Result<Vec<u8>, InternalError>>
        + Send
        + Sync,
>;

struct RouteThunk<S> {
    runner: Runner<S>,
    internal: fn(InternalError) -> Vec<u8>,
}

#[derive(Default)]
pub struct Router<S: Send + Sync + 'static> {
    state: Arc<S>,
    routes: HashMap<RouteId, RouteThunk<S>>,
}

impl<S: Send + Sync + 'static> Router<S> {
    pub fn new(state: S) -> Router<S> {
        Self {
            state: Arc::new(state),
            routes: HashMap::new(),
        }
    }

    pub fn stateless() -> Router<()> {
        Router::new(())
    }

    pub fn route<R: Route, F: Future<Output = R::Response> + Send + 'static>(
        self,
        handler: fn(R::Request, Arc<S>) -> F,
    ) -> Self {
        self.insert::<R, _>(move |bytes, state| {
            Box::pin(async move {
                let req: R::Request = bitcode::decode(bytes).context(InternalSnafu)?;
                Ok(bitcode::encode(&handler(req, state).await))
            })
        })
    }

    pub fn route_stateless<R: Route, F: Future<Output = R::Response> + Send + 'static>(
        self,
        handler: fn(R::Request) -> F,
    ) -> Self {
        self.insert::<R, _>(move |bytes, _| {
            Box::pin(async move {
                let req: R::Request = bitcode::decode(bytes).context(InternalSnafu)?;
                Ok(bitcode::encode(&handler(req).await))
            })
        })
    }

    // The `for<'a>` bound here is what makes closure inference work.
    fn insert<R: Route, G>(mut self, runner: G) -> Self
    where
        G: for<'a> Fn(&'a [u8], Arc<S>) -> ArchBoxFuture<'a, Result<Vec<u8>, InternalError>>
            + Send
            + Sync
            + 'static,
    {
        debug!(
            name = std::any::type_name::<R>(),
            route = hex::encode(R::ID.to_le_bytes()),
            "Registering route"
        );

        self.routes.insert(
            R::ID,
            RouteThunk {
                runner: Box::new(runner),
                internal: |e| bitcode::encode(&<R::Response as FromInternal>::from_internal(e)),
            },
        );

        self
    }

    pub fn build(self) -> Arc<Self> {
        info!(routes = self.routes.len(), "Router built");
        Arc::new(self)
    }

    /// No data if and only if the route is not found.
    pub(crate) async fn run<'a>(&'a self, route_id: RouteId, data: &'a [u8]) -> Option<Vec<u8>> {
        let route_hex = hex::encode(route_id.to_le_bytes());
        let Some(RouteThunk {
            internal: internal_ser,
            runner,
        }) = &self.routes.get(&route_id)
        else {
            debug!(route = route_hex, "Client requested unknown route");
            return None;
        };

        trace!(route = route_hex, len = data.len(), "Running route");

        let res = AssertUnwindSafe(runner(data, Arc::clone(&self.state)))
            .catch_unwind()
            .await;

        let bytes = match res {
            Ok(Ok(bytes)) if bytes.len() + mem::size_of::<MessageId>() <= frame::MAX_FRAME_LEN => {
                bytes
            }
            Ok(Ok(bytes)) => {
                warn!(
                    route = route_hex,
                    len = bytes.len(),
                    "Response too large, returning internal error"
                );
                internal_ser(InternalError {
                    message: "response too large".to_string(),
                })
            }
            Ok(Err(internal)) => internal_ser(internal),
            Err(_) => {
                error!(route = route_hex, "Handler panicked");
                internal_ser(InternalError {
                    message: "server panicked while responding".to_string(),
                })
            }
        };

        trace!(route = route_hex, len = bytes.len(), "Route responded");

        Some(bytes)
    }
}
