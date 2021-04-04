use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::task::{JoinError, JoinHandle};


/// Ensures that all tasks spawned by a future will be canceled when the future
/// (i.e. created by an async fn) is dropped or when it completes.
pub struct JoinGuard<T> {
    inner: JoinHandle<T>,
}

impl<T> Drop for JoinGuard<T> {
    fn drop(&mut self) {
        self.inner.abort();
    }
}

impl<T> Deref for JoinGuard<T> {
    type Target = JoinHandle<T>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> DerefMut for JoinGuard<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<T> Future for JoinGuard<T> {
    type Output = std::result::Result<T, JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        std::pin::Pin::new(&mut self.inner).poll(cx)
    }
}


pub trait JoinHandleExt<T> {
    fn guard(self) -> JoinGuard<T>;
}

impl<T> JoinHandleExt<T> for JoinHandle<T> {
    fn guard(self) -> JoinGuard<T> {
        JoinGuard { inner: self }
    }
}


pub struct ScopeGuard<F: FnOnce()> {
    callback: Option<F>,
}

impl<F: FnOnce()> Drop for ScopeGuard<F> {
    fn drop(&mut self) {
        self.callback.take().unwrap()();
    }
}

pub fn guard<F: FnOnce()>(callback: F) -> ScopeGuard<F> {
    ScopeGuard { callback: Some(callback) }
}


// for logging with dynamic level
macro_rules! event {
    (target: $target:expr, $lvl:expr, $($k:ident).+ = $($fields:tt)* ) => {
        match $lvl {
            ::tracing::Level::ERROR => ::tracing::event!(target: $target, ::tracing::Level::ERROR, $($k).+ = $($fields)*),
            ::tracing::Level::WARN  => ::tracing::event!(target: $target, ::tracing::Level::WARN,  $($k).+ = $($fields)*),
            ::tracing::Level::INFO  => ::tracing::event!(target: $target, ::tracing::Level::INFO,  $($k).+ = $($fields)*),
            ::tracing::Level::DEBUG => ::tracing::event!(target: $target, ::tracing::Level::DEBUG, $($k).+ = $($fields)*),
            ::tracing::Level::TRACE => ::tracing::event!(target: $target, ::tracing::Level::TRACE, $($k).+ = $($fields)*),
        }
    };

    (target: $target:expr, $lvl:expr, $($arg:tt)+ ) => {
        match $lvl {
            ::tracing::Level::ERROR => ::tracing::event!(target: $target, ::tracing::Level::ERROR, $($arg)+),
            ::tracing::Level::WARN  => ::tracing::event!(target: $target, ::tracing::Level::WARN,  $($arg)+),
            ::tracing::Level::INFO  => ::tracing::event!(target: $target, ::tracing::Level::INFO,  $($arg)+),
            ::tracing::Level::DEBUG => ::tracing::event!(target: $target, ::tracing::Level::DEBUG, $($arg)+),
            ::tracing::Level::TRACE => ::tracing::event!(target: $target, ::tracing::Level::TRACE, $($arg)+),
        }
    };

    ($lvl:expr, $($k:ident).+ = $($field:tt)*) => {
        match $lvl {
            ::tracing::Level::ERROR => ::tracing::event!(::tracing::Level::ERROR, $($k).+ = $($field)*),
            ::tracing::Level::WARN  => ::tracing::event!(::tracing::Level::WARN,  $($k).+ = $($field)*),
            ::tracing::Level::INFO  => ::tracing::event!(::tracing::Level::INFO,  $($k).+ = $($field)*),
            ::tracing::Level::DEBUG => ::tracing::event!(::tracing::Level::DEBUG, $($k).+ = $($field)*),
            ::tracing::Level::TRACE => ::tracing::event!(::tracing::Level::TRACE, $($k).+ = $($field)*),
        }
    };

    ( $lvl:expr, $($arg:tt)+ ) => {
        match $lvl {
            ::tracing::Level::ERROR => ::tracing::event!(::tracing::Level::ERROR, $($arg)+),
            ::tracing::Level::WARN  => ::tracing::event!(::tracing::Level::WARN,  $($arg)+),
            ::tracing::Level::INFO  => ::tracing::event!(::tracing::Level::INFO,  $($arg)+),
            ::tracing::Level::DEBUG => ::tracing::event!(::tracing::Level::DEBUG, $($arg)+),
            ::tracing::Level::TRACE => ::tracing::event!(::tracing::Level::TRACE, $($arg)+),
        }
    };
}
