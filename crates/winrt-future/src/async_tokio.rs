use super::*;
use std::future::Future;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

const E_UNEXPECTED: HRESULT = HRESULT(0x8000_FFFFu32 as i32);
const E_ILLEGAL_METHOD_CALL: HRESULT = HRESULT(0x8000_000Eu32 as i32);
const E_ILLEGAL_DELEGATE_ASSIGNMENT: HRESULT = HRESULT(0x8000_0018u32 as i32);

static NEXT_ASYNC_ID: AtomicU32 = AtomicU32::new(1);
static TOKIO_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Returns the shared multi-threaded Tokio runtime driving WinRT async operations.
pub fn runtime() -> &'static tokio::runtime::Runtime {
    TOKIO_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("winrt-tokio")
            .build()
            .expect("failed to initialize winrt-future Tokio runtime")
    })
}

/// Reports incremental progress updates from a running `*WithProgress` Tokio future
/// to any attached WinRT `Progress` delegate (such as C# `IProgress<T>`).
pub struct ProgressReporter<P> {
    emit: Arc<dyn Fn(&P) + Send + Sync>,
}

impl<P> Clone for ProgressReporter<P> {
    fn clone(&self) -> Self {
        Self {
            emit: Arc::clone(&self.emit),
        }
    }
}

impl<P> ProgressReporter<P> {
    /// Emits a progress update to the registered delegate, if any.
    pub fn report(&self, value: P) {
        (self.emit)(&value);
    }

    /// Emits a progress update by reference to the registered delegate, if any.
    pub fn report_ref(&self, value: &P) {
        (self.emit)(value);
    }
}

enum Outcome<O> {
    Finished(Result<O>),
    Canceled,
}

struct State<T: Async> {
    id: u32,
    outcome: Option<Outcome<T::Output>>,
    completed: Option<T::CompletedHandler>,
    completed_assigned: bool,
    abort_handle: Option<tokio::task::AbortHandle>,
}

impl<T: Async> State<T> {
    fn status(&self) -> AsyncStatus {
        match &self.outcome {
            None => AsyncStatus::Started,
            Some(Outcome::Finished(Ok(_))) => AsyncStatus::Completed,
            Some(Outcome::Finished(Err(_))) => AsyncStatus::Error,
            Some(Outcome::Canceled) => AsyncStatus::Canceled,
        }
    }

    fn error_code(&self) -> HRESULT {
        match &self.outcome {
            Some(Outcome::Finished(Err(error))) => error.clone().into(),
            _ => HRESULT(0),
        }
    }

    fn get_results(&self) -> Result<T::Output> {
        match &self.outcome {
            Some(Outcome::Finished(result)) => result.clone(),
            None | Some(Outcome::Canceled) => Err(Error::from_hresult(E_ILLEGAL_METHOD_CALL)),
        }
    }
}

struct SyncState<T: Async>(Mutex<State<T>>);

impl<T: Async> SyncState<T> {
    fn new() -> Self {
        Self(Mutex::new(State {
            id: NEXT_ASYNC_ID.fetch_add(1, Ordering::Relaxed),
            outcome: None,
            completed: None,
            completed_assigned: false,
            abort_handle: None,
        }))
    }

    fn id(&self) -> u32 {
        self.0.lock().unwrap().id
    }

    fn status(&self) -> AsyncStatus {
        self.0.lock().unwrap().status()
    }

    fn error_code(&self) -> HRESULT {
        self.0.lock().unwrap().error_code()
    }

    fn get_results(&self) -> Result<T::Output> {
        self.0.lock().unwrap().get_results()
    }

    fn completed(&self) -> Result<T::CompletedHandler> {
        self.0
            .lock()
            .unwrap()
            .completed
            .clone()
            .ok_or_else(Error::empty)
    }

    fn set_abort_handle(&self, handle: tokio::task::AbortHandle) {
        let mut guard = self.0.lock().unwrap();
        match &guard.outcome {
            None => guard.abort_handle = Some(handle),
            Some(Outcome::Canceled) => {
                drop(guard);
                handle.abort();
            }
            Some(Outcome::Finished(_)) => {}
        }
    }

    fn set_completed(&self, sender: &T, handler: Ref<T::CompletedHandler>) -> Result<()> {
        let mut guard = self.0.lock().unwrap();

        if guard.completed_assigned {
            return Err(Error::from_hresult(E_ILLEGAL_DELEGATE_ASSIGNMENT));
        }

        let status = guard.status();
        let handler = handler.ok()?;
        guard.completed_assigned = true;

        if status == AsyncStatus::Started {
            guard.completed = Some(handler.clone());
        } else {
            drop(guard);
            sender.invoke_completed(handler, status);
        }

        Ok(())
    }

    fn cancel(&self, sender: &T) {
        let mut guard = self.0.lock().unwrap();
        if guard.outcome.is_some() {
            return;
        }
        guard.outcome = Some(Outcome::Canceled);
        let abort = guard.abort_handle.take();
        let completed = guard.completed.take();
        drop(guard);

        if let Some(abort) = abort {
            abort.abort();
        }
        if let Some(completed) = completed {
            sender.invoke_completed(&completed, AsyncStatus::Canceled);
        }
    }

    fn complete(&self, sender: &T, result: Result<T::Output>) {
        let mut guard = self.0.lock().unwrap();
        if guard.outcome.is_some() {
            return;
        }
        guard.outcome = Some(Outcome::Finished(result));
        guard.abort_handle = None;
        let status = guard.status();
        let completed = guard.completed.take();
        drop(guard);

        if let Some(completed) = completed {
            sender.invoke_completed(&completed, status);
        }
    }
}

unsafe impl<T: Async> Send for SyncState<T> {}
unsafe impl<T: Async> Sync for SyncState<T> {}

/// Ensures that even if a spawned future is aborted or panics before calling `complete`,
/// any non-canceled Waiter/CompletedHandler is woken with an error instead of hanging forever.
struct CompletionGuard<T: Async, F: FnOnce(Result<T::Output>)> {
    finish: Option<F>,
    _marker: core::marker::PhantomData<T>,
}

impl<T: Async, F: FnOnce(Result<T::Output>)> CompletionGuard<T, F> {
    fn new(finish: F) -> Self {
        Self {
            finish: Some(finish),
            _marker: core::marker::PhantomData,
        }
    }

    fn complete(mut self, result: Result<T::Output>) {
        if let Some(finish) = self.finish.take() {
            finish(result);
        }
    }
}

impl<T: Async, F: FnOnce(Result<T::Output>)> Drop for CompletionGuard<T, F> {
    fn drop(&mut self) {
        if let Some(finish) = self.finish.take() {
            finish(Err(Error::new(
                E_UNEXPECTED,
                "async task dropped without completing",
            )));
        }
    }
}

struct Action(SyncState<IAsyncAction>);
implement_decl! {
    impl Action as Action_Impl: [IAsyncAction, IAsyncInfo]
}

struct Operation<T>(SyncState<IAsyncOperation<T>>)
where
    T: RuntimeType + 'static;
implement_decl! {
    impl<T> Operation as Operation_Impl: [
        IAsyncOperation<T>,
        IAsyncInfo,
    ]
    where T: RuntimeType + 'static
}

struct ActionWithProgress<P>
where
    P: RuntimeType + 'static,
{
    state: SyncState<IAsyncActionWithProgress<P>>,
    progress: Mutex<Option<AsyncActionProgressHandler<P>>>,
}
unsafe impl<P: RuntimeType + 'static> Send for ActionWithProgress<P> {}
unsafe impl<P: RuntimeType + 'static> Sync for ActionWithProgress<P> {}
implement_decl! {
    impl<P> ActionWithProgress as ActionWithProgress_Impl: [
        IAsyncActionWithProgress<P>,
        IAsyncInfo,
    ]
    where P: RuntimeType + 'static
}

struct OperationWithProgress<T, P>
where
    T: RuntimeType + 'static,
    P: RuntimeType + 'static,
{
    state: SyncState<IAsyncOperationWithProgress<T, P>>,
    progress: Mutex<Option<AsyncOperationProgressHandler<T, P>>>,
}
unsafe impl<T: RuntimeType + 'static, P: RuntimeType + 'static> Send
    for OperationWithProgress<T, P>
{
}
unsafe impl<T: RuntimeType + 'static, P: RuntimeType + 'static> Sync
    for OperationWithProgress<T, P>
{
}
implement_decl! {
    impl<T, P> OperationWithProgress as OperationWithProgress_Impl: [
        IAsyncOperationWithProgress<T, P>,
        IAsyncInfo,
    ]
    where T: RuntimeType + 'static, P: RuntimeType + 'static
}

impl IAsyncInfo_Impl for Action_Impl {
    fn Id(&self) -> Result<u32> {
        Ok(self.0.id())
    }
    fn Status(&self) -> Result<AsyncStatus> {
        Ok(self.0.status())
    }
    fn ErrorCode(&self) -> Result<HRESULT> {
        Ok(self.0.error_code())
    }
    fn Cancel(&self) -> Result<()> {
        self.0.cancel(&self.as_interface());
        Ok(())
    }
    fn Close(&self) -> Result<()> {
        Ok(())
    }
}

impl<T: RuntimeType> IAsyncInfo_Impl for Operation_Impl<T> {
    fn Id(&self) -> Result<u32> {
        Ok(self.0.id())
    }
    fn Status(&self) -> Result<AsyncStatus> {
        Ok(self.0.status())
    }
    fn ErrorCode(&self) -> Result<HRESULT> {
        Ok(self.0.error_code())
    }
    fn Cancel(&self) -> Result<()> {
        self.0.cancel(&self.as_interface());
        Ok(())
    }
    fn Close(&self) -> Result<()> {
        Ok(())
    }
}

impl<P: RuntimeType> IAsyncInfo_Impl for ActionWithProgress_Impl<P> {
    fn Id(&self) -> Result<u32> {
        Ok(self.state.id())
    }
    fn Status(&self) -> Result<AsyncStatus> {
        Ok(self.state.status())
    }
    fn ErrorCode(&self) -> Result<HRESULT> {
        Ok(self.state.error_code())
    }
    fn Cancel(&self) -> Result<()> {
        self.state.cancel(&self.as_interface());
        Ok(())
    }
    fn Close(&self) -> Result<()> {
        Ok(())
    }
}

impl<T: RuntimeType, P: RuntimeType> IAsyncInfo_Impl for OperationWithProgress_Impl<T, P> {
    fn Id(&self) -> Result<u32> {
        Ok(self.state.id())
    }
    fn Status(&self) -> Result<AsyncStatus> {
        Ok(self.state.status())
    }
    fn ErrorCode(&self) -> Result<HRESULT> {
        Ok(self.state.error_code())
    }
    fn Cancel(&self) -> Result<()> {
        self.state.cancel(&self.as_interface());
        Ok(())
    }
    fn Close(&self) -> Result<()> {
        Ok(())
    }
}

impl IAsyncAction_Impl for Action_Impl {
    fn SetCompleted(&self, handler: Ref<AsyncActionCompletedHandler>) -> Result<()> {
        self.0.set_completed(&self.as_interface(), handler)
    }
    fn Completed(&self) -> Result<AsyncActionCompletedHandler> {
        self.0.completed()
    }
    fn GetResults(&self) -> Result<()> {
        self.0.get_results()
    }
}

impl<T: RuntimeType> IAsyncOperation_Impl<T> for Operation_Impl<T> {
    fn SetCompleted(&self, handler: Ref<AsyncOperationCompletedHandler<T>>) -> Result<()> {
        self.0.set_completed(&self.as_interface(), handler)
    }
    fn Completed(&self) -> Result<AsyncOperationCompletedHandler<T>> {
        self.0.completed()
    }
    fn GetResults(&self) -> Result<T> {
        self.0.get_results()
    }
}

impl<P: RuntimeType> IAsyncActionWithProgress_Impl<P> for ActionWithProgress_Impl<P> {
    fn SetCompleted(&self, handler: Ref<AsyncActionWithProgressCompletedHandler<P>>) -> Result<()> {
        self.state.set_completed(&self.as_interface(), handler)
    }
    fn Completed(&self) -> Result<AsyncActionWithProgressCompletedHandler<P>> {
        self.state.completed()
    }
    fn GetResults(&self) -> Result<()> {
        self.state.get_results()
    }
    fn SetProgress(&self, handler: Ref<AsyncActionProgressHandler<P>>) -> Result<()> {
        *self.progress.lock().unwrap() = handler.ok().ok().cloned();
        Ok(())
    }
    fn Progress(&self) -> Result<AsyncActionProgressHandler<P>> {
        self.progress
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(Error::empty)
    }
}

impl<T: RuntimeType, P: RuntimeType> IAsyncOperationWithProgress_Impl<T, P>
    for OperationWithProgress_Impl<T, P>
{
    fn SetCompleted(
        &self,
        handler: Ref<AsyncOperationWithProgressCompletedHandler<T, P>>,
    ) -> Result<()> {
        self.state.set_completed(&self.as_interface(), handler)
    }
    fn Completed(&self) -> Result<AsyncOperationWithProgressCompletedHandler<T, P>> {
        self.state.completed()
    }
    fn GetResults(&self) -> Result<T> {
        self.state.get_results()
    }
    fn SetProgress(&self, handler: Ref<AsyncOperationProgressHandler<T, P>>) -> Result<()> {
        *self.progress.lock().unwrap() = handler.ok().ok().cloned();
        Ok(())
    }
    fn Progress(&self) -> Result<AsyncOperationProgressHandler<T, P>> {
        self.progress
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(Error::empty)
    }
}

impl IAsyncAction {
    /// Runs a blocking closure on Tokio's blocking pool and returns an `IAsyncAction`.
    pub fn spawn<F>(f: F) -> Self
    where
        F: FnOnce() -> Result<()> + Send + 'static,
    {
        Self::spawn_future(async move {
            tokio::task::spawn_blocking(f)
                .await
                .map_err(|e| Error::new(E_UNEXPECTED, e.to_string()))?
        })
    }

    /// Drives a Rust `Future<Output = Result<()>>` on the shared Tokio runtime as an `IAsyncAction`.
    pub fn spawn_future<F>(future: F) -> Self
    where
        F: Future<Output = Result<()>> + Send + 'static,
    {
        let object = ComObject::new(Action(SyncState::new()));
        let interface: Self = object.to_interface();
        let task_object = object.clone();

        let handle = runtime().spawn(async move {
            let guard = CompletionGuard::<IAsyncAction, _>::new(|res| {
                task_object.0.complete(&task_object.as_interface(), res);
            });
            let result = future.await;
            guard.complete(result);
        });
        object.0.set_abort_handle(handle.abort_handle());

        interface
    }
}

impl<T: RuntimeType> IAsyncOperation<T> {
    /// Runs a blocking closure on Tokio's blocking pool and returns an `IAsyncOperation<T>`.
    pub fn spawn<F>(f: F) -> Self
    where
        T: Send,
        F: FnOnce() -> Result<T> + Send + 'static,
    {
        Self::spawn_future(async move {
            tokio::task::spawn_blocking(f)
                .await
                .map_err(|e| Error::new(E_UNEXPECTED, e.to_string()))?
        })
    }

    /// Drives a Rust `Future<Output = Result<T>>` on the shared Tokio runtime as an `IAsyncOperation<T>`.
    pub fn spawn_future<F>(future: F) -> Self
    where
        F: Future<Output = Result<T>> + Send + 'static,
    {
        let object = ComObject::new(Operation(SyncState::new()));
        let interface: Self = object.to_interface();
        let task_object = object.clone();

        let handle = runtime().spawn(async move {
            let guard = CompletionGuard::<IAsyncOperation<T>, _>::new(|res| {
                task_object.0.complete(&task_object.as_interface(), res);
            });
            let result = future.await;
            guard.complete(result);
        });
        object.0.set_abort_handle(handle.abort_handle());

        interface
    }
}

impl<P: RuntimeType> IAsyncActionWithProgress<P> {
    /// Runs a blocking closure on Tokio's blocking pool and returns an `IAsyncActionWithProgress<P>`.
    pub fn spawn<F>(f: F) -> Self
    where
        F: FnOnce() -> Result<()> + Send + 'static,
    {
        Self::spawn_with_progress(move |_| async move {
            tokio::task::spawn_blocking(f)
                .await
                .map_err(|e| Error::new(E_UNEXPECTED, e.to_string()))?
        })
    }

    /// Drives a Tokio future created with a [`ProgressReporter<P>`] handle.
    pub fn spawn_with_progress<M, F>(make_future: M) -> Self
    where
        M: FnOnce(ProgressReporter<P>) -> F,
        F: Future<Output = Result<()>> + Send + 'static,
    {
        let object = ComObject::new(ActionWithProgress {
            state: SyncState::new(),
            progress: Mutex::new(None),
        });
        let interface: Self = object.to_interface();

        let progress_object = object.clone();
        let reporter = ProgressReporter {
            emit: Arc::new(move |value: &P| {
                let handler = progress_object.progress.lock().unwrap().clone();
                if let Some(handler) = handler {
                    let iface = progress_object.as_interface::<IAsyncActionWithProgress<P>>();
                    _ = handler.Invoke(&*iface, Some(value));
                }
            }),
        };

        let future = make_future(reporter);
        let task_object = object.clone();
        let handle = runtime().spawn(async move {
            let guard = CompletionGuard::<IAsyncActionWithProgress<P>, _>::new(|res| {
                task_object
                    .state
                    .complete(&task_object.as_interface(), res);
            });
            let result = future.await;
            guard.complete(result);
        });
        object.state.set_abort_handle(handle.abort_handle());

        interface
    }
}

impl<T: RuntimeType, P: RuntimeType> IAsyncOperationWithProgress<T, P> {
    /// Runs a blocking closure on Tokio's blocking pool and returns an `IAsyncOperationWithProgress<T, P>`.
    pub fn spawn<F>(f: F) -> Self
    where
        T: Send,
        F: FnOnce() -> Result<T> + Send + 'static,
    {
        Self::spawn_with_progress(move |_| async move {
            tokio::task::spawn_blocking(f)
                .await
                .map_err(|e| Error::new(E_UNEXPECTED, e.to_string()))?
        })
    }

    /// Drives a Tokio future created with a [`ProgressReporter<P>`] handle.
    pub fn spawn_with_progress<M, F>(make_future: M) -> Self
    where
        M: FnOnce(ProgressReporter<P>) -> F,
        F: Future<Output = Result<T>> + Send + 'static,
    {
        let object = ComObject::new(OperationWithProgress {
            state: SyncState::new(),
            progress: Mutex::new(None),
        });
        let interface: Self = object.to_interface();

        let progress_object = object.clone();
        let reporter = ProgressReporter {
            emit: Arc::new(move |value: &P| {
                let handler = progress_object.progress.lock().unwrap().clone();
                if let Some(handler) = handler {
                    let iface =
                        progress_object.as_interface::<IAsyncOperationWithProgress<T, P>>();
                    _ = handler.Invoke(&*iface, Some(value));
                }
            }),
        };

        let future = make_future(reporter);
        let task_object = object.clone();
        let handle = runtime().spawn(async move {
            let guard = CompletionGuard::<IAsyncOperationWithProgress<T, P>, _>::new(|res| {
                task_object
                    .state
                    .complete(&task_object.as_interface(), res);
            });
            let result = future.await;
            guard.complete(result);
        });
        object.state.set_abort_handle(handle.abort_handle());

        interface
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    #[test]
    fn ready_and_spawn_future_roundtrip() {
        let ready = IAsyncOperation::ready(Ok(42));
        assert_eq!(ready.Status().unwrap(), AsyncStatus::Completed);
        assert_eq!(ready.join().unwrap(), 42);

        let op = IAsyncOperation::spawn_future(async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            Ok(99_i32)
        });
        assert_eq!(op.join().unwrap(), 99);
        assert_eq!(op.Status().unwrap(), AsyncStatus::Completed);
    }

    #[test]
    fn tokio_await_winrt_operation() {
        let op = IAsyncOperation::spawn_future(async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            Ok(21_i32)
        });
        let doubled = runtime().block_on(async move { Ok::<_, Error>(op.await? * 2) }).unwrap();
        assert_eq!(doubled, 42);
    }

    #[test]
    fn cancel_aborts_tokio_task_and_notifies() {
        let completed_status = Arc::new(Mutex::new(None));
        let completed_status_clone = completed_status.clone();

        let action = IAsyncAction::spawn_future(async {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok(())
        });
        action
            .SetCompleted(&AsyncActionCompletedHandler::new(move |_, status| {
                *completed_status_clone.lock().unwrap() = Some(status);
                Ok(())
            }))
            .unwrap();

        action.Cancel().unwrap();
        assert_eq!(action.Status().unwrap(), AsyncStatus::Canceled);
        assert_eq!(*completed_status.lock().unwrap(), Some(AsyncStatus::Canceled));
        assert!(action.GetResults().is_err());
    }

    #[test]
    fn progress_reporter_fires_handler() {
        let sum = Arc::new(AtomicUsize::new(0));
        let sum_clone = sum.clone();

        let op = IAsyncOperationWithProgress::<i32, u32>::spawn_with_progress(|reporter| async move {
            for step in 1..=3 {
                tokio::time::sleep(Duration::from_millis(5)).await;
                reporter.report(step);
            }
            Ok(123)
        });
        op.SetProgress(&AsyncOperationProgressHandler::<i32, u32>::new(
            move |_, value| {
                sum_clone.fetch_add(value as usize, Ordering::Relaxed);
                Ok(())
            },
        ))
        .unwrap();

        assert_eq!(op.join().unwrap(), 123);
        assert_eq!(sum.load(Ordering::Relaxed), 6);
    }
}
