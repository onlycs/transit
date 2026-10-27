use std::{error::Error, fmt};

use snafu::IntoError;

pub(super) mod private {
    use std::marker::PhantomData;

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
        __Phantom(std::convert::Infallible, PhantomData<fn(E)>),
    }

    pub enum ConvertToErrorViaErrorCtx<S, E> {
        _ConvertToErrorViaError(E),

        #[doc(hidden)]
        __Phantom(std::convert::Infallible, PhantomData<fn(S)>),
    }

    pub enum ConvertToErrorViaDisplayCtx<S, E> {
        _ConvertToErrorViaDisplay(E),

        #[doc(hidden)]
        __Phantom(std::convert::Infallible, PhantomData<fn(S)>),
    }
}

pub use private::{
    ConvertToErrorViaDisplayCtx::_ConvertToErrorViaDisplay,
    ConvertToErrorViaErrorCtx::_ConvertToErrorViaError,
};
use tracing::warn;

impl<E> IntoError<private::InternalError> for private::InternalErrorCtx<E>
where
    E: Into<Box<dyn Error + Send + Sync>>,
{
    type Source = E;

    #[track_caller]
    fn into_error(self, source: E) -> private::InternalError {
        let source: Box<dyn Error + Send + Sync> = source.into();

        warn!(
            at = %std::panic::Location::caller(),
            "Returning an internal error. Full report:\n{}",
            snafu::Report::from_error(&*source).to_string()
        );

        private::InternalError {
            message: source.to_string(),
        }
    }
}

impl<S, E> IntoError<E> for private::ConvertToErrorViaErrorCtx<S, E>
where
    E: snafu::ErrorCompat + std::error::Error,
    S: Into<Box<dyn Error + Send + Sync>>,
{
    type Source = S;

    #[track_caller]
    fn into_error(self, source: S) -> E {
        let source: Box<dyn Error + Send + Sync> = source.into();

        let res = match self {
            Self::_ConvertToErrorViaError(e) => e,
            Self::__Phantom(never, _) => match never {},
        };

        warn!(
            at = %std::panic::Location::caller(),
            returned = %res,
            "Returning an error. Full report:\n{}",
            snafu::Report::from_error(&*source).to_string()
        );

        res
    }
}

impl<S, E> IntoError<E> for private::ConvertToErrorViaDisplayCtx<S, E>
where
    E: snafu::ErrorCompat + std::error::Error,
    S: fmt::Display,
{
    type Source = S;

    #[track_caller]
    fn into_error(self, source: S) -> E {
        let res = match self {
            Self::_ConvertToErrorViaDisplay(e) => e,
            Self::__Phantom(never, _) => match never {},
        };

        warn!(
            at = %std::panic::Location::caller(),
            returned = %res,
            "Returning an error. Source message:\n{source}",
        );

        res
    }
}

#[macro_export]
macro_rules! InternalError {
    // CASE: dev just wants a new InternalError
    (ctx(none), $fmt:literal $($rest:tt)*) => {{
        let message = ::std::format!($fmt $($rest)*);

        ::tracing::warn!(
            at = %std::panic::Location::caller(),
            returned = message,
            "Returning an internal error."
        );

        $crate::InternalError { message }
    }};

    (ctx(none), $e:expr) => {
        $crate::InternalError!(ctx(none), "{}", $e)
    };

    // CASE: dev wants to create a new InternalError, but has and wants to emit a source, but the source is not StdError
    (ctx(display $ctx:expr), $fmt:literal $($rest:tt)*) => {{
        let message = ::std::format!($fmt $($rest)*);

        ::tracing::warn!(
            at = %std::panic::Location::caller(),
            returned = message,
            "Returning an internal error. Full report:\n{}",
            $ctx
        );

        $crate::InternalError { message }
    }};

    (ctx(display $ctx:expr), $e:expr) => {
        $crate::InternalError!(ctx(display $ctx), "{}", $e)
    };

    // CASE: dev wants to create a new InternalError, but has and wants to emit a source
    (ctx($ctx:expr), $fmt:literal $($rest:tt)*) => {{
        let message = ::std::format!($fmt $($rest)*);

        ::tracing::warn!(
            at = %std::panic::Location::caller(),
            returned = message,
            "Returning an internal error. Full report:\n{}",
            ::snafu::Report::from_error(&$ctx).to_string()
        );

        $crate::InternalError { message }
    }};

    (ctx($ctx:expr), $e:expr) => {
        $crate::InternalError!(ctx($ctx), "{}", $e)
    };
}

#[macro_export]
macro_rules! InternalErrorContext {
    // CASE: dev wants to use .context() with this macro, but source is not StdError
    (via(display), $fmt:literal $($rest:tt)*) => {
        $crate::TransitErrorContext!(via(display), $crate::InternalError { message: ::std::format!($fmt $($rest)*) })
    };

    (via(display), $e:expr) => {
        $crate::InternalErrorContext!(via(display), "{}", $e)
    };

    // CASE: dev wants to use .context() with this macro
    ($fmt:literal $($rest:tt)*) => {
        $crate::TransitErrorContext!($crate::InternalError { message: ::std::format!($fmt $($rest)*) })
    };

    ($e:expr) => {
        $crate::InternalErrorContext!("{}", $e)
    };
}

#[macro_export]
macro_rules! TransitErrorContext {
    (via(display), $into:expr) => {
        $crate::error::_ConvertToErrorViaDisplay($into)
    };

    ($into:expr) => {
        $crate::error::_ConvertToErrorViaError($into)
    };
}
