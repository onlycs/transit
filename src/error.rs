use std::error::Error;

use snafu::{ErrorCompat, IntoError};

mod private {
    use transit_macros::error_shard;

    #[error_shard("Internal error: {message}")]
    #[snafu(module)]
    pub struct InternalError {
        pub message: String,
    }

    pub enum InternalErrorCtx<E> {
        InternalSnafu,

        #[doc(hidden)]
        __Phantom(std::convert::Infallible, std::marker::PhantomData<fn(E)>),
    }

    pub enum InternalErrorMessageCtx<S: AsRef<str>, E> {
        _InternalErrorMessage(S),

        #[doc(hidden)]
        __Phantom(std::convert::Infallible, std::marker::PhantomData<fn(E)>),
    }
}

pub use private::{
    InternalError, InternalErrorCtx::InternalSnafu, InternalErrorMessageCtx::_InternalErrorMessage,
};
use tracing::warn;

impl<T, E> IntoError<T> for private::InternalErrorCtx<E>
where
    T: From<InternalError> + ErrorCompat + Error,
    E: Into<Box<dyn Error + Send + Sync>>,
{
    type Source = E;

    fn into_error(self, source: E) -> T {
        let source: Box<dyn Error + Send + Sync> = source.into();

        warn!(
            "Returning an internal error: {source}. Full report:\n{}",
            snafu::Report::from_error(&*source).to_string()
        );

        T::from(InternalError {
            message: source.to_string(),
        })
    }
}

impl<T: From<InternalError> + ErrorCompat + Error, S: AsRef<str>, E> IntoError<T>
    for private::InternalErrorMessageCtx<S, E>
{
    type Source = E;

    fn into_error(self, _: E) -> T {
        let message = match self {
            private::InternalErrorMessageCtx::_InternalErrorMessage(s) => s.as_ref().to_string(),
            private::InternalErrorMessageCtx::__Phantom(never, _) => match never {},
        };

        T::from(InternalError { message })
    }
}

#[macro_export]
macro_rules! InternalErrorMessage {
    ($($args:tt)*) => {
        $crate::error::_InternalErrorMessage(::std::format!($($args)*))
    };
}
