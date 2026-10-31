use bitcode::{DecodeOwned, Encode};
use transit_reflect::TypeHash;

use crate::{InternalError, frame::RouteId};

// replace with try_trait_v2 when stable
pub trait FromInternal {
    fn from_internal(error: InternalError) -> Self;
}

impl<T, E: From<InternalError>> FromInternal for Result<T, E> {
    fn from_internal(error: InternalError) -> Self {
        Err(error.into())
    }
}

pub trait Route {
    const NAME: &'static str;

    type Request: Encode + DecodeOwned + TypeHash + Send + Sync + 'static;
    type Response: Encode + DecodeOwned + TypeHash + FromInternal + Send + Sync + 'static;
}

pub trait RouteExt: Route {
    const ID: RouteId;
}

impl<R: Route> RouteExt for R {
    const ID: RouteId = transit_reflect::route_id::<Self::Request, Self::Response>(Self::NAME);
}
