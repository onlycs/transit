use std::{error::Error, fmt};

use snafu::IntoError;

mod private {
    use snafu::NoneError;
    use transit_macros::error_shard;
    #[cfg(target_family = "wasm")]
    use wasm_bindgen::prelude::wasm_bindgen;

    #[error_shard("Internal server error: {message}")]
    #[snafu(module)]
    #[cfg_attr(target_family = "wasm", wasm_bindgen(getter_with_clone))]
    pub struct InternalError {
        pub message: String,
    }

    impl snafu::FromString for InternalError {
        type Source = NoneError;

        fn with_source(_: Self::Source, message: String) -> Self {
            Self { message }
        }

        fn without_source(message: String) -> Self {
            Self { message }
        }
    }

    pub enum InternalErrorCtx<E> {
        InternalSnafu,

        #[doc(hidden)]
        __Phantom(std::convert::Infallible, std::marker::PhantomData<fn(E)>),
    }

    pub enum InternalErrorMessageViaErrorCtx<S: AsRef<str>, E> {
        _InternalErrorMessageViaError(S),

        #[doc(hidden)]
        __Phantom(std::convert::Infallible, std::marker::PhantomData<fn(E)>),
    }

    pub enum InternalErrorMessageViaDisplayCtx<S: AsRef<str>, E> {
        _InternalErrorMessageViaDisplay(S),

        #[doc(hidden)]
        __Phantom(std::convert::Infallible, std::marker::PhantomData<fn(E)>),
    }
}

pub use private::{
    InternalError, InternalErrorCtx::InternalSnafu,
    InternalErrorMessageViaDisplayCtx::_InternalErrorMessageViaDisplay,
    InternalErrorMessageViaErrorCtx::_InternalErrorMessageViaError,
};
use tracing::warn;

impl<E> IntoError<InternalError> for private::InternalErrorCtx<E>
where
    E: Into<Box<dyn Error + Send + Sync>>,
{
    type Source = E;

    #[track_caller]
    fn into_error(self, source: E) -> InternalError {
        let source: Box<dyn Error + Send + Sync> = source.into();

        warn!(
            at = %std::panic::Location::caller(),
            "Returning an internal error. Full report:\n{}",
            snafu::Report::from_error(&*source).to_string()
        );

        InternalError {
            message: source.to_string(),
        }
    }
}

impl<S: AsRef<str>, E> IntoError<InternalError> for private::InternalErrorMessageViaErrorCtx<S, E>
where
    E: Into<Box<dyn Error + Send + Sync>>,
{
    type Source = E;

    #[track_caller]
    fn into_error(self, source: E) -> InternalError {
        let source: Box<dyn Error + Send + Sync> = source.into();

        let message = match self {
            Self::_InternalErrorMessageViaError(s) => s.as_ref().to_string(),
            Self::__Phantom(never, _) => match never {},
        };

        warn!(
            at = %std::panic::Location::caller(),
            reason = message,
            "Returning an internal error. Full report:\n{}",
            snafu::Report::from_error(&*source).to_string()
        );

        InternalError { message }
    }
}

impl<S: AsRef<str>, E> IntoError<InternalError> for private::InternalErrorMessageViaDisplayCtx<S, E>
where
    E: fmt::Display,
{
    type Source = E;

    #[track_caller]
    fn into_error(self, source: E) -> InternalError {
        let message = match self {
            Self::_InternalErrorMessageViaDisplay(s) => s.as_ref().to_string(),
            Self::__Phantom(never, _) => match never {},
        };

        warn!(
            at = %std::panic::Location::caller(),
            reason = message,
            "Returning an internal error. Full report:\n{source}"
        );

        InternalError { message }
    }
}

#[macro_export]
macro_rules! InternalErrorMessage {
    (ctx(none), $e:expr) => {
        $crate::InternalErrorMessage!(ctx(none), "{}", $e)
    };

    (ctx(none), $($args:tt)*) => {{
        let message = ::std::format!($($args)*);

        ::tracing::warn!(
            at = %std::panic::Location::caller(),
            reason = message,
            "Returning an internal error."
        );

        $crate::error::InternalError { message }
    }};

    (ctx(display), $e:expr) => {
        $crate::error::_InternalErrorMessageViaDisplay(::std::format!("{}", $e))
    };

    (ctx(display), $($args:tt)*) => {
        $crate::error::_InternalErrorMessageViaDisplay(::std::format!($($args)*))
    };

    (ctx($ctx:expr), $e:expr) => {
        $crate::InternalErrorMessage!(ctx($ctx), "{}", $e)
    };

    (ctx($ctx:expr), $($args:tt)*) => {{
        let message = ::std::format!($($args)*);

        ::tracing::warn!(
            at = %std::panic::Location::caller(),
            reason = message,
            "Returning an internal error. Full report:\n{}",
            ::snafu::Report::from_error(&$ctx).to_string()
        );

        $crate::error::InternalError { message }
    }};

    (ctx(display $ctx:expr), $e:expr) => {
        $crate::InternalErrorMessage!(ctx(display $ctx), "{}", $e)
    };

    (ctx(display $ctx:expr), $($args:tt)*) => {{
        let message = ::std::format!($($args)*);

        ::tracing::warn!(
            at = %std::panic::Location::caller(),
            reason = message,
            "Returning an internal error. Full report:\n{}",
            $ctx
        );

        $crate::error::InternalError { message }
    }};

    ($e:expr) => {
        $crate::error::_InternalErrorMessageViaError(::std::format!("{}", $e))
    };

    ($($args:tt)*) => {
        $crate::error::_InternalErrorMessageViaError(::std::format!($($args)*))
    };
}
