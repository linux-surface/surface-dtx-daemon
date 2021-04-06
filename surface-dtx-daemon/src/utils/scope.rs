
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
