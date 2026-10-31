use std::pin::Pin;

#[cfg(not(feature = "futures-not-send"))]
pub trait ArchSend: Send {}
#[cfg(not(feature = "futures-not-send"))]
impl<T: Send> ArchSend for T {}
#[cfg(feature = "futures-not-send")]
pub trait ArchSend {}
#[cfg(feature = "futures-not-send")]
impl<T> ArchSend for T {}

#[cfg(not(feature = "futures-not-send"))]
pub type ArchBoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
#[cfg(feature = "futures-not-send")]
pub type ArchBoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;
