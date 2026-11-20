//! Bridge between GPUI's executor and stonks-core's tokio-based services.
//!
//! reqwest needs a tokio reactor; GPUI runs its own dispatcher. A small
//! multi-thread runtime owns all core work; `spawn_core` hands back a future
//! any executor can await.

use std::future::Future;
use std::sync::LazyLock;

static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime for stonks-core services")
});

pub fn spawn_core<F>(future: F) -> impl Future<Output = F::Output> + Send
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let (sender, receiver) = futures_channel::oneshot::channel();
    RUNTIME.spawn(async move {
        let _ = sender.send(future.await);
    });
    async move { receiver.await.expect("core service task panicked") }
}
