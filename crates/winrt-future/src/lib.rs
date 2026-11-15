#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals)]

//! Cross-platform WinRT async runtime (`IAsyncAction`, `IAsyncOperation`, and `*WithProgress`)
//! backed by a shared multi-threaded Tokio runtime.
//!
//! Designed as a drop-in Linux/macOS replacement for `windows-future`:
//! - Exposes all standard WinRT async COM interfaces, delegates, and [`AsyncStatus`].
//! - Implements [`IntoFuture`](std::future::IntoFuture) so Rust `async fn` can `.await` WinRT async objects.
//! - Provides `spawn_future` / `spawn_with_progress` (and `spawn` / `ready`) to drive Tokio futures
//!   from WinRT components with `IAsyncInfo::Cancel` abort support.

mod r#async;
mod async_ready;
mod async_tokio;
#[expect(non_snake_case, non_camel_case_types, clippy::upper_case_acronyms)]
mod bindings;
mod future;
mod waiter;
mod when;

use waiter::*;
use windows_core::*;

pub use r#async::Async;
pub use async_tokio::{ProgressReporter, runtime};
pub use bindings::*;
pub use future::AsyncFuture;
