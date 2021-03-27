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
