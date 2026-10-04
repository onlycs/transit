use crate::{InternalError, frame};

pub trait Route {
    const ID: frame::RouteId;

    type Request: bitcode::Encode + bitcode::DecodeOwned + Send + Sync + 'static;
    type Response: bitcode::Encode
        + bitcode::DecodeOwned
        + From<InternalError>
        + Send
        + Sync
        + 'static;
}
