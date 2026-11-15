use super::*;
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, Ordering},
};

struct Once<T> {
    taken: AtomicBool,
    value: UnsafeCell<Option<T>>,
}

// Only the winning take accesses the value through a shared reference.
unsafe impl<T: Send> Sync for Once<T> {}

impl<T> Once<T> {
    fn new(value: T) -> Self {
        Self {
            taken: AtomicBool::new(false),
            value: UnsafeCell::new(Some(value)),
        }
    }

    fn take(&self) -> Option<T> {
        if self.taken.swap(true, Ordering::Relaxed) {
            None
        } else {
            unsafe { (*self.value.get()).take() }
        }
    }
}

/// Represents a WinRT async execution object (`IAsyncAction`, `IAsyncActionWithProgress`,
/// `IAsyncOperation`, `IAsyncOperationWithProgress`).
pub trait Async: Interface {
    /// The type of value produced on completion.
    type Output: Clone;

    /// The type of the delegate used for completion notification.
    type CompletedHandler: Interface;

    /// Sets the handler to invoke when execution completes. This handler can only be set once.
    fn set_completed<F: Fn(&Self) + Send + Sync + 'static>(&self, handler: F) -> Result<()>;

    /// Calls the given handler with the current object and status.
    fn invoke_completed(&self, handler: &Self::CompletedHandler, status: AsyncStatus);

    /// Returns the value produced on completion.
    fn get_results(&self) -> Result<Self::Output>;

    /// Gets the current status of async execution.
    fn status(&self) -> Result<AsyncStatus>;

    /// Blocks the current thread until async execution finishes and returns its result.
    fn join(&self) -> Result<Self::Output> {
        if self.status()? == AsyncStatus::Started {
            let (waiter, signaler) = Waiter::new()?;
            let signaler = Once::new(signaler);
            self.set_completed(move |_| {
                if let Some(signaler) = signaler.take() {
                    signaler.signal();
                }
            })?;
            waiter.wait();
        }
        self.get_results()
    }

    /// Calls `op(result)` when async execution completes.
    fn when<F>(&self, op: F) -> Result<()>
    where
        F: FnOnce(Result<Self::Output>) + Send + 'static,
    {
        if self.status()? == AsyncStatus::Started {
            let op = Once::new(op);
            self.set_completed(move |sender| {
                if let Some(op) = op.take() {
                    op(sender.get_results());
                }
            })?;
        } else {
            op(self.get_results());
        }
        Ok(())
    }
}

impl Async for IAsyncAction {
    type Output = ();
    type CompletedHandler = AsyncActionCompletedHandler;

    fn set_completed<F: Fn(&Self) + Send + Sync + 'static>(&self, handler: F) -> Result<()> {
        self.SetCompleted(&AsyncActionCompletedHandler::new(move |sender, _| {
            handler(sender.ok()?);
            Ok(())
        }))
    }

    fn invoke_completed(&self, handler: &Self::CompletedHandler, status: AsyncStatus) {
        _ = handler.Invoke(self, status);
    }

    fn get_results(&self) -> Result<Self::Output> {
        self.GetResults()
    }

    fn status(&self) -> Result<AsyncStatus> {
        self.Status()
    }
}

impl<T: RuntimeType> Async for IAsyncOperation<T> {
    type Output = T;
    type CompletedHandler = AsyncOperationCompletedHandler<T>;

    fn set_completed<F: Fn(&Self) + Send + Sync + 'static>(&self, handler: F) -> Result<()> {
        self.SetCompleted(&AsyncOperationCompletedHandler::new(move |sender, _| {
            handler(sender.ok()?);
            Ok(())
        }))
    }

    fn invoke_completed(&self, handler: &Self::CompletedHandler, status: AsyncStatus) {
        _ = handler.Invoke(self, status);
    }

    fn get_results(&self) -> Result<Self::Output> {
        self.GetResults()
    }

    fn status(&self) -> Result<AsyncStatus> {
        self.Status()
    }
}

impl<P: RuntimeType> Async for IAsyncActionWithProgress<P> {
    type Output = ();
    type CompletedHandler = AsyncActionWithProgressCompletedHandler<P>;

    fn set_completed<F: Fn(&Self) + Send + Sync + 'static>(&self, handler: F) -> Result<()> {
        self.SetCompleted(&AsyncActionWithProgressCompletedHandler::new(
            move |sender, _| {
                handler(sender.ok()?);
                Ok(())
            },
        ))
    }

    fn invoke_completed(&self, handler: &Self::CompletedHandler, status: AsyncStatus) {
        _ = handler.Invoke(self, status);
    }

    fn get_results(&self) -> Result<Self::Output> {
        self.GetResults()
    }

    fn status(&self) -> Result<AsyncStatus> {
        self.Status()
    }
}

impl<T: RuntimeType, P: RuntimeType> Async for IAsyncOperationWithProgress<T, P> {
    type Output = T;
    type CompletedHandler = AsyncOperationWithProgressCompletedHandler<T, P>;

    fn set_completed<F: Fn(&Self) + Send + Sync + 'static>(&self, handler: F) -> Result<()> {
        self.SetCompleted(&AsyncOperationWithProgressCompletedHandler::new(
            move |sender, _| {
                handler(sender.ok()?);
                Ok(())
            },
        ))
    }

    fn invoke_completed(&self, handler: &Self::CompletedHandler, status: AsyncStatus) {
        _ = handler.Invoke(self, status);
    }

    fn get_results(&self) -> Result<Self::Output> {
        self.GetResults()
    }

    fn status(&self) -> Result<AsyncStatus> {
        self.Status()
    }
}
