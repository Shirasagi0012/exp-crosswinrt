use super::*;
use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

/// Adapts a WinRT asynchronous operation to [`Future`].
pub struct AsyncFuture<A: Async> {
    inner: A,
    status: IAsyncInfo,
    waker: Option<Arc<AsyncWaker>>,
}

struct AsyncWaker(Mutex<Option<Waker>>);

impl AsyncWaker {
    fn new(waker: &Waker) -> Self {
        Self(Mutex::new(Some(waker.clone())))
    }

    fn register(&self, waker: &Waker) {
        if self
            .0
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|current| current.will_wake(waker))
        {
            return;
        }
        let waker = waker.clone();
        let previous = self.0.lock().unwrap().replace(waker);
        drop(previous);
    }

    fn wake(&self) {
        let waker = self.0.lock().unwrap().take();
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

impl<A: Async> AsyncFuture<A> {
    fn new(inner: A) -> Self {
        Self {
            status: inner.cast().unwrap(),
            inner,
            waker: None,
        }
    }
}

unsafe impl<A: Async> Send for AsyncFuture<A> {}
unsafe impl<A: Async> Sync for AsyncFuture<A> {}
impl<A: Async> Unpin for AsyncFuture<A> {}

impl<A: Async> Future for AsyncFuture<A> {
    type Output = Result<A::Output>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        if self.status.Status()? != AsyncStatus::Started {
            return Poll::Ready(self.inner.get_results());
        }

        if let Some(shared_waker) = &self.waker {
            shared_waker.register(cx.waker());

            if self.status.Status()? != AsyncStatus::Started {
                return Poll::Ready(self.inner.get_results());
            }
        } else {
            let shared_waker = Arc::new(AsyncWaker::new(cx.waker()));
            self.waker = Some(shared_waker.clone());

            self.inner.set_completed(move |_| {
                shared_waker.wake();
            })?;
        };

        Poll::Pending
    }
}

impl IntoFuture for IAsyncAction {
    type Output = Result<()>;
    type IntoFuture = AsyncFuture<Self>;

    fn into_future(self) -> Self::IntoFuture {
        AsyncFuture::new(self)
    }
}

impl<T: RuntimeType> IntoFuture for IAsyncOperation<T> {
    type Output = Result<T>;
    type IntoFuture = AsyncFuture<Self>;

    fn into_future(self) -> Self::IntoFuture {
        AsyncFuture::new(self)
    }
}

impl<P: RuntimeType> IntoFuture for IAsyncActionWithProgress<P> {
    type Output = Result<()>;
    type IntoFuture = AsyncFuture<Self>;

    fn into_future(self) -> Self::IntoFuture {
        AsyncFuture::new(self)
    }
}

impl<T: RuntimeType, P: RuntimeType> IntoFuture for IAsyncOperationWithProgress<T, P> {
    type Output = Result<T>;
    type IntoFuture = AsyncFuture<Self>;

    fn into_future(self) -> Self::IntoFuture {
        AsyncFuture::new(self)
    }
}
