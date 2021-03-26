use std::future::Future;
use std::pin::Pin;

use tokio::sync::mpsc::{channel, Receiver, Sender};


pub type Task<E> = Pin<Box<dyn Future<Output=Result<(), E>> + Send>>;

pub struct TaskQueue<E> {
    rx: Receiver<Task<E>>,
}

impl<E> TaskQueue<E> {
    pub fn new() -> (Self, Sender<Task<E>>) {
        Self::with_capacity(32)
    }

    pub fn with_capacity(size: usize) -> (Self, Sender<Task<E>>) {
        let (tx, rx) = channel(size);

        (TaskQueue { rx }, tx)
    }

    pub async fn run(&mut self) -> Result<(), E> {
        while let Some(task) = self.rx.recv().await {
            task.await?;
        }

        Ok(())
    }
}
