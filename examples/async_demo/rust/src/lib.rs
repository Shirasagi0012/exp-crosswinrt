#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals)]

pub mod bindings;

use bindings::server::async_worker_factory;
use bindings::{AsyncWorker, IAsyncWorker_Impl};
use std::time::Duration;
use windows_core::{implement, Error, HRESULT, HSTRING, Ref, Result, RuntimeName};
use winrt_future::{IAsyncAction, IAsyncOperation, IAsyncOperationWithProgress};

const DISP_E_DIVBYZERO: HRESULT = HRESULT(0x8002_0012u32 as i32);
const E_INVALIDARG: HRESULT = HRESULT(0x8007_0057u32 as i32);

#[implement(AsyncWorker)]
pub struct AsyncWorkerImpl;

impl Default for AsyncWorkerImpl {
    fn default() -> Self {
        let _ = winrt_future::runtime();
        Self
    }
}

impl IAsyncWorker_Impl for AsyncWorkerImpl_Impl {
    fn DelayAsync(&self, millis: u32) -> Result<IAsyncAction> {
        Ok(IAsyncAction::spawn_future(async move {
            tokio::time::sleep(Duration::from_millis(millis as u64)).await;
            Ok(())
        }))
    }

    fn FibonacciAsync(&self, n: u32) -> Result<IAsyncOperation<u64>> {
        if n > 93 {
            return Ok(IAsyncOperation::ready(Err(Error::from_hresult(E_INVALIDARG))));
        }
        Ok(IAsyncOperation::spawn(move || {
            let mut a = 0_u64;
            let mut b = 1_u64;
            for _ in 0..n {
                let next = a.wrapping_add(b);
                a = b;
                b = next;
            }
            Ok(a)
        }))
    }

    fn ProcessStepsAsync(
        &self,
        steps: u32,
        step_delay_millis: u32,
    ) -> Result<IAsyncOperationWithProgress<HSTRING, u32>> {
        Ok(IAsyncOperationWithProgress::spawn_with_progress(
            move |reporter| async move {
                for step in 1..=steps {
                    tokio::time::sleep(Duration::from_millis(step_delay_millis as u64)).await;
                    reporter.report(step);
                }
                Ok(HSTRING::from(format!("Completed {steps} steps on Tokio")))
            },
        ))
    }

    fn SumPairAsync(
        &self,
        left: Ref<IAsyncOperation<i32>>,
        right: Ref<IAsyncOperation<i32>>,
    ) -> Result<IAsyncOperation<i32>> {
        let left = left.ok()?.clone();
        let right = right.ok()?.clone();

        Ok(IAsyncOperation::spawn_future(async move {
            let (a, b) = tokio::join!(left, right);
            Ok(a? + b?)
        }))
    }

    fn FailAsync(&self, immediate: bool) -> Result<IAsyncOperation<i32>> {
        if immediate {
            Err(Error::from_hresult(E_INVALIDARG))
        } else {
            Ok(IAsyncOperation::spawn_future(async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                Err(Error::from_hresult(DISP_E_DIVBYZERO))
            }))
        }
    }
}

winrt_cross_pal::export_winrt_component! {
    AsyncWorker::NAME => async_worker_factory::<AsyncWorkerImpl>(),
}
