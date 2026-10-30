//! The executor the window's subscriptions and tasks run on: tokio, with two worker threads.
//! Why two, not tokio's one per core: the work is timers, event filters and the wait for a file
//! dialog, microseconds each, and an idle worker is not free: the scheduler wakes idle workers
//! to look for work to steal, so with 24 of them every timer tick woke several threads.

use std::future::Future;
use std::io;

use tokio::runtime::{Builder, Runtime};

/// Worker threads: one for the work, one spare so a slow future never holds up a timer.
const WORKERS: usize = 2;

pub struct Executor(Runtime);

impl iced::Executor for Executor {
    fn new() -> Result<Self, io::Error> {
        Builder::new_multi_thread()
            .worker_threads(WORKERS)
            .thread_name("dawsome-tasks")
            .enable_all()
            .build()
            .map(Self)
    }

    fn spawn(&self, future: impl Future<Output = ()> + Send + 'static) {
        drop(self.0.spawn(future));
    }

    fn block_on<T>(&self, future: impl Future<Output = T>) -> T {
        self.0.block_on(future)
    }

    fn enter<R>(&self, f: impl FnOnce() -> R) -> R {
        let _guard = self.0.enter();
        f()
    }
}
