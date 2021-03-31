use std::future::Future;
use std::pin::Pin;

use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::sync::mpsc::error::SendError;


pub type Task<E> = Pin<Box<dyn Future<Output=Result<(), E>> + Send>>;

pub struct TaskQueue<E> {
    rx: Receiver<Task<E>>,
}

impl<E> TaskQueue<E> {
    pub async fn run(&mut self) -> Result<(), E> {
        while let Some(task) = self.rx.recv().await {
            task.await?;
        }

        Ok(())
    }
}


pub struct TaskSender<E> {
    tx: Sender<Task<E>>,
}

impl<E> TaskSender<E> {
    pub async fn submit<T>(&self, task: T) -> Result<(), SendError<Task<E>>>
    where
        T: Future<Output=Result<(), E>> + Send + 'static
    {
        self.tx.send(Box::pin(task)).await
    }
}


pub fn new<E>() -> (TaskQueue<E>, TaskSender<E>) {
    with_capacity(32)
}

pub fn with_capacity<E>(size: usize) -> (TaskQueue<E>, TaskSender<E>) {
    let (tx, rx) = channel(size);

    (TaskQueue { rx }, TaskSender { tx })
}
