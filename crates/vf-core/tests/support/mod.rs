//! Shared helper for the F3a port-contract tests (VFL-314, T4 slice of
//! VFL-44 for F3a / VFL-310).
//!
//! `vf-core` carries no async runtime (architecture §A1.3), so these tests
//! drive the `PortFuture` values the ports return by hand instead of
//! reaching for `tokio`. Every future under test in this pack either is, or
//! is built from, `std::future::ready` (the same primitive `vf-core`'s own
//! `TenantArtifactStore::refuse` helper uses, and the only kind of future a
//! hand-written test double in this pack produces), so it is always `Ready`
//! on the first poll. A single poll with a no-op waker is therefore enough;
//! a `Pending` result means a test double is not what this pack assumes it
//! is, not that the caller should wait and retry.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

struct NoopWaker;

impl Wake for NoopWaker {
    fn wake(self: Arc<Self>) {}
}

/// Polls `fut` once and returns its output.
///
/// # Panics
///
/// If `fut` is not `Ready` on the first poll.
pub fn block_on<T>(mut fut: Pin<Box<dyn Future<Output = T> + Send + '_>>) -> T {
    let waker = Waker::from(Arc::new(NoopWaker));
    let mut cx = Context::from_waker(&waker);
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!(
            "block_on: future was not Ready on the first poll; this helper \
             only drives futures that resolve synchronously, which is \
             everything under test in this pack"
        ),
    }
}
