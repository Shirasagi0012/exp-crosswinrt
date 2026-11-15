#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals)]

pub mod bindings;

use bindings::server::{Calculator_Factory, calculator_factory};
use bindings::{CalcOp, CalcRecord, Calculator, ICalcObserver, ICalculator_Impl};
use std::sync::Mutex;
use windows_core::{implement, Error, HRESULT, HSTRING, Ref, Result, RuntimeName};
use winrt_server_shims::WinRtArray;

const DISP_E_DIVBYZERO: HRESULT = HRESULT(0x8002_0012u32 as i32);
const E_INVALIDARG: HRESULT = HRESULT(0x8007_0057u32 as i32);

#[derive(Default)]
struct CalcState {
    memory: f64,
    history: Vec<CalcRecord>,
}

#[implement(Calculator)]
pub struct CalculatorImpl {
    state: Mutex<CalcState>,
}

impl CalculatorImpl {
    fn with_memory(initial_memory: f64) -> Self {
        Self {
            state: Mutex::new(CalcState {
                memory: initial_memory,
                history: Vec::new(),
            }),
        }
    }

    fn apply_op(op: CalcOp, left: f64, right: f64) -> Result<f64> {
        match op {
            CalcOp::Add => Ok(left + right),
            CalcOp::Subtract => Ok(left - right),
            CalcOp::Multiply => Ok(left * right),
            CalcOp::Divide => {
                if right == 0.0 {
                    Err(Error::from_hresult(DISP_E_DIVBYZERO))
                } else {
                    Ok(left / right)
                }
            }
            _ => Err(Error::from_hresult(E_INVALIDARG)),
        }
    }

    fn op_symbol(op: CalcOp) -> &'static str {
        match op {
            CalcOp::Add => "+",
            CalcOp::Subtract => "-",
            CalcOp::Multiply => "*",
            CalcOp::Divide => "/",
            _ => "?",
        }
    }
}

impl ICalculator_Impl for CalculatorImpl_Impl {
    fn Memory(&self) -> Result<f64> {
        Ok(self.state.lock().unwrap().memory)
    }

    fn Evaluate(&self, op: CalcOp, left: f64, right: f64) -> Result<f64> {
        let result = CalculatorImpl::apply_op(op, left, right)?;
        let mut guard = self.state.lock().unwrap();
        guard.history.push(CalcRecord {
            op,
            left,
            right,
            result,
        });
        Ok(result)
    }

    fn EvaluateWithObserver(
        &self,
        op: CalcOp,
        left: f64,
        right: f64,
        observer: Ref<ICalcObserver>,
    ) -> Result<f64> {
        let result = self.Evaluate(op, left, right)?;
        if let Some(cb) = observer.as_ref() {
            cb.OnEvaluated(CalcRecord {
                op,
                left,
                right,
                result,
            })?;
        }
        Ok(result)
    }

    fn EvaluateBatch(&self, op: CalcOp, values: &[f64]) -> Result<f64> {
        let Some((&first, rest)) = values.split_first() else {
            return Ok(0.0);
        };
        let mut acc = first;
        for &val in rest {
            acc = CalculatorImpl::apply_op(op, acc, val)?;
        }
        self.state.lock().unwrap().history.push(CalcRecord {
            op,
            left: first,
            right: rest.last().copied().unwrap_or(0.0),
            result: acc,
        });
        Ok(acc)
    }

    fn MemoryAdd(&self, value: f64) -> Result<f64> {
        let mut guard = self.state.lock().unwrap();
        guard.memory += value;
        Ok(guard.memory)
    }

    fn MemoryClear(&self) -> Result<()> {
        self.state.lock().unwrap().memory = 0.0;
        Ok(())
    }

    fn GetHistory(&self) -> Result<WinRtArray<CalcRecord>> {
        let guard = self.state.lock().unwrap();
        WinRtArray::from_slice(&guard.history)
    }

    fn FormatLastRecord(&self) -> Result<HSTRING> {
        let guard = self.state.lock().unwrap();
        let text = match guard.history.last() {
            Some(rec) => format!(
                "{} {} {} = {}",
                rec.left,
                CalculatorImpl::op_symbol(rec.op),
                rec.right,
                rec.result
            ),
            None => "<empty>".to_string(),
        };
        Ok(HSTRING::from(text))
    }
}

impl Calculator_Factory for CalculatorImpl {
    fn ActivateInstance() -> Result<Calculator> {
        Ok(Self::with_memory(0.0).into())
    }

    fn CreateCalculator(initial_memory: f64) -> Result<Calculator> {
        Ok(Self::with_memory(initial_memory).into())
    }

    fn QuickEval(op: CalcOp, left: f64, right: f64) -> Result<f64> {
        Self::apply_op(op, left, right)
    }
}

winrt_cross_pal::export_winrt_component! {
    Calculator::NAME => calculator_factory::<CalculatorImpl>(),
}
