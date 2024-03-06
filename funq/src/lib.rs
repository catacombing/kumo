use std::any::Any;
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

pub use funq_derive::callbacks;
use rustix::event::{eventfd, EventfdFlags};
use rustix::io::{read, write};

/// Callback-based event queue.
pub struct Queue<S> {
    // Queue handles for thread-safe callbacks.
    mt_rx: Receiver<MtFun<S>>,
    mt_tx: Sender<MtFun<S>>,

    // Queue handles for thread-local callbacks.
    st_rx: Receiver<StFun<S>>,
    st_tx: Sender<StFun<S>>,

    // FD to notify about any updates.
    waker: Arc<OwnedFd>,
}

impl<S> Queue<S> {
    /// Create a new event queue.
    pub fn new() -> Result<Self, io::Error> {
        // Create callback channels.
        let (mt_tx, mt_rx) = mpsc::channel();
        let (st_tx, st_rx) = mpsc::channel();

        // Create eventfd for waking up event loops.
        let waker = Arc::new(eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?);

        Ok(Self { waker, mt_tx, mt_rx, st_tx, st_rx })
    }

    /// Get a handle for dispatching thread-safe callbacks.
    ///
    /// To dispatch callbacks which are not thread-safe, see
    /// [`Self::local_handle`] instead.
    pub fn handle(&self) -> MtQueueHandle<S> {
        MtQueueHandle { tx: self.mt_tx.clone(), waker: self.waker.clone() }
    }

    /// Get a handle for dispatching thread-local callbacks.
    ///
    /// This allows for function parameters which cannot be sent across thread
    /// boundaries, but sending the handle itself is not safe either.
    pub fn local_handle(&self) -> StQueueHandle<S> {
        StQueueHandle { tx: self.st_tx.clone(), waker: self.waker.clone() }
    }

    /// Dispatch all pending callbacks.
    ///
    /// This will never block and should be called whenever [`Self::fd`] becomes
    /// readable.
    pub fn dispatch(&self, state: &mut S) -> Result<(), io::Error> {
        // Empty all pending callbacks.
        while let Ok(callback) = self.mt_rx.try_recv() {
            (callback.fun)(state, callback.args);
        }
        while let Ok(callback) = self.st_rx.try_recv() {
            (callback.fun)(state, callback.args);
        }

        // Drain waker.
        let mut buf = [0u8; 8];
        read(&self.waker, &mut buf)?;

        Ok(())
    }

    /// Get handle for the underlying file descriptor.
    ///
    /// This can be used to integrate into event loops, since the file
    /// descriptor will be readable whenever [`Self::try_run`]` should be
    /// called.
    pub fn fd(&self) -> BorrowedFd<'_> {
        self.waker.as_fd()
    }
}

/// Event queue handle for thread-safe callbacks.
pub struct MtQueueHandle<S> {
    tx: Sender<MtFun<S>>,
    waker: Arc<OwnedFd>,
}

impl<S> MtQueueHandle<S> {
    /// Send a new callback to the event queue.
    pub fn send(&self, fun: MtFun<S>) {
        let _ = self.tx.send(fun);
        let _ = write(&self.waker, &1u64.to_ne_bytes());
    }
}

impl<S> Clone for MtQueueHandle<S> {
    fn clone(&self) -> Self {
        Self { tx: self.tx.clone(), waker: self.waker.clone() }
    }
}

/// Event queue handle for thread-local callbacks.
pub struct StQueueHandle<S> {
    tx: Sender<StFun<S>>,
    waker: Arc<OwnedFd>,
}

impl<S> StQueueHandle<S> {
    /// Send a new callback to the event queue.
    pub fn send(&self, fun: StFun<S>) {
        let _ = self.tx.send(fun);
        let _ = write(&self.waker, &1u64.to_ne_bytes());
    }
}

impl<S> Clone for StQueueHandle<S> {
    fn clone(&self) -> Self {
        Self { tx: self.tx.clone(), waker: self.waker.clone() }
    }
}

/// Function signature for thread-safe dispatch.
type MtFunTy<S> = Box<dyn Fn(&mut S, Vec<Box<dyn Any + Send + Sync>>) + Send + Sync>;

/// Thread-safe callback.
pub struct MtFun<S> {
    pub fun: MtFunTy<S>,
    pub args: Vec<Box<dyn Any + Send + Sync>>,
}

/// Function signature for thread-local dispatch.
type StFunTy<S> = Box<dyn Fn(&mut S, Vec<Box<dyn Any>>)>;

/// Thread-local callback.
pub struct StFun<S> {
    pub fun: StFunTy<S>,
    pub args: Vec<Box<dyn Any>>,
}
