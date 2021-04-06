use std::future::Future;
use std::pin::Pin;

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::mpsc::error::SendError;

use tracing::trace;


pub type Task<E> = Pin<Box<dyn Future<Output=Result<(), E>> + Send>>;


#[derive(Debug)]
pub struct TaskQueue<E> {
    rx: UnboundedReceiver<Task<E>>,
}

impl<E> TaskQueue<E> {
    pub async fn run(&mut self) -> Result<(), E> {
        while let Some(task) = self.rx.recv().await {
            trace!(target: "sdtxd::tq", "running next task");
            let result = task.await;
            trace!(target: "sdtxd::tq", "task completed");
            result?;
        }

        Ok(())
    }
}


#[derive(Debug, Clone)]
pub struct TaskSender<E> {
    tx: UnboundedSender<Task<E>>,
}

impl<E> TaskSender<E> {
    pub fn submit<T>(&self, task: T) -> Result<(), SendError<Task<E>>>
    where
        T: Future<Output=Result<(), E>> + Send + 'static
    {
        trace!(target: "sdtxd::tq", "submitting new task");
        self.tx.send(Box::pin(task))
    }
}


pub fn new<E>() -> (TaskQueue<E>, TaskSender<E>) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    (TaskQueue { rx }, TaskSender { tx })
}
