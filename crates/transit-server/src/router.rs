use std::{collections::HashMap, mem, panic::AssertUnwindSafe, sync::Arc};

use futures::FutureExt;
use snafu::ResultExt;
use tracing::{debug, error, info, trace, warn};
use transit_core::{
    InternalError, InternalSnafu, Route,
    frame::{self, MessageId, RouteId},
    route::FromInternal,
    send::ArchBoxFuture,
};

#[derive(Clone, Copy)]
struct FnErased(*const ());

// SAFETY: FnErased should be a function pointer without a type
// function pointers are always safe to send and sync
unsafe impl Send for FnErased {}
unsafe impl Sync for FnErased {}

struct StateErased(*const ());

// SAFETY: `S` is bounded by Send+Sync inside the struct
unsafe impl Send for StateErased {}
unsafe impl Sync for StateErased {}

type FnRunnerResponse<'a> = ArchBoxFuture<'a, Result<Vec<u8>, InternalError>>;
type FnRunner = for<'a> fn(&'a [u8], FnErased, StateErased) -> FnRunnerResponse<'a>;
type FnEncodeInternal = fn(InternalError) -> Vec<u8>;

/// I refuse to deal with dyn Fn objects, sue me.
#[derive(Clone, Copy)]
pub struct RouteThunk {
    erased: FnErased,
    internal: FnEncodeInternal,
    runner: FnRunner,
}

#[derive(Default)]
pub struct Router<S: Send + Sync + 'static> {
    state: Arc<S>,
    routes: HashMap<RouteId, RouteThunk>,
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
        mut self,
        handler: fn(R::Request, Arc<S>) -> F,
    ) -> Self {
        debug!(
            name = std::any::type_name::<R>(),
            route = hex::encode(R::ID.to_le_bytes()),
            "Registering route"
        );

        self.routes.insert(
            R::ID,
            RouteThunk {
                runner: |bytes, FnErased(erased), StateErased(state)| {
                    let handler: fn(R::Request, Arc<S>) -> F = unsafe { mem::transmute(erased) };
                    let state: Arc<S> = unsafe { Arc::from_raw(state as *const S) };

                    trace!(
                        name = std::any::type_name::<R>(),
                        route = hex::encode(R::ID.to_le_bytes()),
                        "Beginning route"
                    );

                    Box::pin(async move {
                        let req: R::Request = bitcode::decode(bytes).context(InternalSnafu)?;
                        let res = handler(req, state).await;
                        Ok(bitcode::encode(&res))
                    })
                },
                internal: |error| {
                    bitcode::encode(&<R::Response as FromInternal>::from_internal(error))
                },
                erased: FnErased(handler as *const ()),
            },
        );

        self
    }

    pub fn route_stateless<R: Route, F: Future<Output = R::Response> + Send + 'static>(
        mut self,
        handler: fn(R::Request) -> F,
    ) -> Self {
        debug!(
            name = std::any::type_name::<R>(),
            route = hex::encode(R::ID.to_le_bytes()),
            "Registering stateless route"
        );

        self.routes.insert(
            R::ID,
            RouteThunk {
                runner: |bytes, FnErased(erased), _| {
                    let handler: fn(R::Request) -> F = unsafe { mem::transmute(erased) };

                    trace!(
                        name = std::any::type_name::<R>(),
                        route = hex::encode(R::ID.to_le_bytes()),
                        "Beginning route"
                    );

                    Box::pin(async move {
                        let req: R::Request = bitcode::decode(bytes).context(InternalSnafu)?;
                        let res = handler(req).await;
                        Ok(bitcode::encode(&res))
                    })
                },
                internal: |error| {
                    bitcode::encode(&<R::Response as FromInternal>::from_internal(error))
                },
                erased: FnErased(handler as *const ()),
            },
        );

        self
    }

    pub fn build(self) -> Arc<Self> {
        info!(routes = self.routes.len(), "Router built");
        Arc::new(self)
    }

    /// No data if and only if the route is not found.
    pub async fn run<'a>(&'a self, route_id: RouteId, data: &'a [u8]) -> Option<Vec<u8>> {
        let route_hex = hex::encode(route_id.to_le_bytes());
        let Some(&RouteThunk {
            erased,
            internal: internal_ser,
            runner,
        }) = self.routes.get(&route_id)
        else {
            debug!(route = route_hex, "Client requested unknown route");
            return None;
        };

        trace!(route = route_hex, len = data.len(), "Running route");

        let state = Arc::into_raw(Arc::clone(&self.state));
        let state = StateErased(state as *const ());

        let res = AssertUnwindSafe(runner(data, erased, state))
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
