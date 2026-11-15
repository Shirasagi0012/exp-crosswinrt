use std::sync::{Arc, Condvar, Mutex};

/// Cross-platform single-use event waiter replacing Win32 `CreateEventW`/`WaitForSingleObject`.
pub(crate) struct Waiter(Arc<(Mutex<bool>, Condvar)>);

pub(crate) struct WaiterSignaler(Arc<(Mutex<bool>, Condvar)>);

unsafe impl Send for WaiterSignaler {}

impl Waiter {
    pub fn new() -> windows_core::Result<(Self, WaiterSignaler)> {
        let pair = Arc::new((Mutex::new(false), Condvar::new()));
        Ok((Self(pair.clone()), WaiterSignaler(pair)))
    }

    pub fn wait(self) {
        let (lock, cvar) = &*self.0;
        let mut ready = lock.lock().unwrap();
        while !*ready {
            ready = cvar.wait(ready).unwrap();
        }
    }
}

impl WaiterSignaler {
    pub fn signal(&self) {
        let (lock, cvar) = &*self.0;
        let mut ready = lock.lock().unwrap();
        *ready = true;
        cvar.notify_all();
    }
}
