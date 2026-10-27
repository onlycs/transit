use crate::{InternalError, frame};

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
    const ID: frame::RouteId;

    type Request: bitcode::Encode + bitcode::DecodeOwned + Send + Sync + 'static;
    type Response: bitcode::Encode + bitcode::DecodeOwned + FromInternal + Send + Sync + 'static;
}
