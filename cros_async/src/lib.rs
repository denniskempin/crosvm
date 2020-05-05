// Copyright 2020 The Chromium OS Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! An Executor and future combinators based on operations that block on file descriptors.
//!
//! This crate is meant to be used with the `futures-rs` crate that provides further combinators
//! and utility functions to combine and manage futures. All futures will run until they block on a
//! file descriptor becoming readable or writable. Facilities are provided to register future
//! wakers based on such events.
//!
//! # Running top-level futures.
//!
//! Use helper functions based the desired behavior of your application.
//!
//! ## Running one future.
//!
//! If there is only one top-level future to run, use the [`run_one`](fn.run_one.html) function.
//!
//! ## Completing one of several futures.
//!
//! If there are several top level tasks that should run until any one completes, use the "select"
//! family of executor constructors. These return an [`Executor`](trait.Executor.html) whose `run`
//! function will return when the first future completes. The uncompleted futures will also be
//! returned so they can be run further or otherwise cleaned up. These functions are inspired by
//! the `select_all` function from futures-rs, but built to be run inside an FD based executor and
//! to poll only when necessary. See the docs for [`select2`](fn.select2.html),
//! [`select3`](fn.select3.html), [`select4`](fn.select4.html), and [`select5`](fn.select5.html).
//!
//! ## Completing all of several futures.
//!
//! If there are several top level tasks that all need to be completed, use the "complete" family
//! of executor constructors. These return an [`Executor`](trait.Executor.html) whose `run`
//! function will return only once all the futures passed to it have completed. These functions are
//! inspired by the `join_all` function from futures-rs, but built to be run inside an FD based
//! executor and to poll only when necessary. See the docs for [`complete2`](fn.complete2.html),
//! [`complete3`](fn.complete3.html), [`complete4`](fn.complete4.html), and
//! [`complete5`](fn.complete5.html).
//!
//! # Implementing new FD-based futures.
//!
//! When building futures to be run in an `Executor` framework, use the following helper functions
//! to perform common tasks:
//!
//! [`add_read_waker`](fn.add_read_waker.html) - Used to associate a provided FD becoming readable
//! with the future being woken. Used before returning Poll::Pending from a future that waits until
//! an FD is writable.
//!
//! [`add_write_waker`](fn.add_write_waker.html) - Used to associate a provided FD becoming
//! writable with the future being woken. Used before returning Poll::Pending from a future that
//! waits until an FD is readable.
//!
//! [`add_future`](fn.add_future.html) - Used to add a new future to the top-level list of running
//! futures.

mod complete;
mod executor;
mod fd_executor;
mod select;
mod uring_executor;
mod waker;

pub use executor::{Executor, WakerToken};
pub use select::SelectResult;

use executor::{FutureList, RunOne};
use fd_executor::FdExecutor;
use uring_executor::URingExecutor;

use std::fmt::{self, Display};
use std::future::Future;
use std::os::unix::io::RawFd;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::task::Waker;

#[derive(Debug)]
pub enum Error {
    /// Error from the FD executor.
    FdExecutor(fd_executor::Error),
    /// Error from the uring executor.
    URingExecutor(uring_executor::Error),
}
pub type Result<T> = std::result::Result<T, Error>;

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::Error::*;

        match self {
            FdExecutor(e) => write!(f, "Failure in the FD executor: {}", e),
            URingExecutor(e) => write!(f, "Failure in the uring executor: {}", e),
        }
    }
}

// Checks if the uring executor is available and if it us use it.
// Caches the result so that the check is only run once.
// Useful for falling back to the FD executor on pre-uring kernels.
fn use_uring() -> bool {
    const UNKNOWN: u32 = 0;
    const URING: u32 = 1;
    const FD: u32 = 2;
    static USE_URING: AtomicU32 = AtomicU32::new(UNKNOWN);
    match USE_URING.load(Ordering::Relaxed) {
        UNKNOWN => {
            if uring_executor::supported() {
                USE_URING.store(URING, Ordering::Relaxed);
                true
            } else {
                USE_URING.store(FD, Ordering::Relaxed);
                false
            }
        }
        URING => true,
        FD => false,
        _ => unreachable!("invalid use uring state"),
    }
}

// Runs an executor with the given future list.
// Chooses the uring executor if available, otherwise falls back to the FD executor.
fn run_executor<T: FutureList>(future_list: T) -> Result<T::Output> {
    if use_uring() {
        URingExecutor::new(future_list)
            .and_then(|mut ex| ex.run())
            .map_err(Error::URingExecutor)
    } else {
        FdExecutor::new(future_list)
            .and_then(|mut ex| ex.run())
            .map_err(Error::FdExecutor)
    }
}

/// Creates a FdExecutor that runs one future to completion.
///
///  # Example
///
///    ```
///    use cros_async::run_one;
///
///    let fut = async { 55 };
///    assert_eq!(55, run_one(Box::pin(fut)).unwrap());
///    ```
pub fn run_one<F: Future + Unpin>(fut: F) -> Result<F::Output> {
    run_executor(RunOne::new(fut))
}

// Select helpers to run until any future completes.

/// Creates an executor that runs the two given futures until one completes, returning a tuple
/// containing the result of the finished future and the still pending future.
///
///  # Example
///
///    ```
///    use cros_async::{Executor, select2, SelectResult};
///    use futures::future::pending;
///    use futures::pin_mut;
///
///    let first = async {5};
///    let second = async {let () = pending().await;};
///    pin_mut!(first);
///    pin_mut!(second);
///    match select2(first, second) {
///        Ok((SelectResult::Finished(5), SelectResult::Pending(_second))) => (),
///        _ => panic!("Select didn't return the first future"),
///    };
///    ```
pub fn select2<F1: Future + Unpin, F2: Future + Unpin>(
    f1: F1,
    f2: F2,
) -> Result<(SelectResult<F1>, SelectResult<F2>)> {
    run_executor(select::Select2::new(f1, f2))
}

/// Creates an executor that runs the three given futures until one or more completes, returning a
/// tuple containing the result of the finished future(s) and the still pending future(s).
///
///  # Example
///
///    ```
///    use cros_async::{Executor, select3, SelectResult};
///    use futures::future::pending;
///    use futures::pin_mut;
///
///    let first = async {4};
///    let second = async {let () = pending().await;};
///    let third = async {5};
///    pin_mut!(first);
///    pin_mut!(second);
///    pin_mut!(third);
///    match select3(first, second, third) {
///        Ok((SelectResult::Finished(4),
///            SelectResult::Pending(_second),
///            SelectResult::Finished(5))) => (),
///        _ => panic!("Select didn't return the futures"),
///    };
///    ```
pub fn select3<F1: Future + Unpin, F2: Future + Unpin, F3: Future + Unpin>(
    f1: F1,
    f2: F2,
    f3: F3,
) -> Result<(SelectResult<F1>, SelectResult<F2>, SelectResult<F3>)> {
    run_executor(select::Select3::new(f1, f2, f3))
}

/// Creates an executor that runs the four given futures until one or more completes, returning a
/// tuple containing the result of the finished future(s) and the still pending future(s).
///
///  # Example
///
///    ```
///    use cros_async::{Executor, select4, SelectResult};
///    use futures::future::pending;
///    use futures::pin_mut;
///
///    let first = async {4};
///    let second = async {let () = pending().await;};
///    let third = async {5};
///    let fourth = async {let () = pending().await;};
///    pin_mut!(first);
///    pin_mut!(second);
///    pin_mut!(third);
///    pin_mut!(fourth);
///    match select4(first, second, third, fourth) {
///        Ok((SelectResult::Finished(4), SelectResult::Pending(_second),
///            SelectResult::Finished(5), SelectResult::Pending(_fourth))) => (),
///        _ => panic!("Select didn't return the futures"),
///    };
///    ```
pub fn select4<F1: Future + Unpin, F2: Future + Unpin, F3: Future + Unpin, F4: Future + Unpin>(
    f1: F1,
    f2: F2,
    f3: F3,
    f4: F4,
) -> Result<(
    SelectResult<F1>,
    SelectResult<F2>,
    SelectResult<F3>,
    SelectResult<F4>,
)> {
    run_executor(select::Select4::new(f1, f2, f3, f4))
}

/// Creates an executor that runs the five given futures until one or more completes, returning a
/// tuple containing the result of the finished future(s) and the still pending future(s).
///
///  # Example
///
///    ```
///    use cros_async::{Executor, select5, SelectResult};
///    use futures::future::pending;
///    use futures::pin_mut;
///
///    let first = async {4};
///    let second = async {let () = pending().await;};
///    let third = async {5};
///    let fourth = async {let () = pending().await;};
///    let fifth = async {6};
///    pin_mut!(first);
///    pin_mut!(second);
///    pin_mut!(third);
///    pin_mut!(fourth);
///    pin_mut!(fifth);
///    match select5(first, second, third, fourth, fifth) {
///        Ok((SelectResult::Finished(4), SelectResult::Pending(_second),
///            SelectResult::Finished(5), SelectResult::Pending(_fourth),
///            SelectResult::Finished(6))) => (),
///        _ => panic!("Select didn't return the futures"),
///    };
///    ```
pub fn select5<
    F1: Future + Unpin,
    F2: Future + Unpin,
    F3: Future + Unpin,
    F4: Future + Unpin,
    F5: Future + Unpin,
>(
    f1: F1,
    f2: F2,
    f3: F3,
    f4: F4,
    f5: F5,
) -> Result<(
    SelectResult<F1>,
    SelectResult<F2>,
    SelectResult<F3>,
    SelectResult<F4>,
    SelectResult<F5>,
)> {
    run_executor(select::Select5::new(f1, f2, f3, f4, f5))
}

/// Creates an executor that runs the six given futures until one or more completes, returning a
/// tuple containing the result of the finished future(s) and the still pending future(s).
///
///  # Example
///
///    ```
///    use cros_async::{Executor, select6, SelectResult};
///    use futures::future::pending;
///    use futures::pin_mut;
///
///    let first = async {1};
///    let second = async {let () = pending().await;};
///    let third = async {3};
///    let fourth = async {let () = pending().await;};
///    let fifth = async {5};
///    let sixth = async {6};
///    pin_mut!(first);
///    pin_mut!(second);
///    pin_mut!(third);
///    pin_mut!(fourth);
///    pin_mut!(fifth);
///    pin_mut!(sixth);
///    match select6(first, second, third, fourth, fifth, sixth) {
///        Ok((SelectResult::Finished(1), SelectResult::Pending(_second),
///            SelectResult::Finished(3), SelectResult::Pending(_fourth),
///            SelectResult::Finished(5), SelectResult::Finished(6))) => (),
///        _ => panic!("Select didn't return the futures"),
///    };
///    ```
pub fn select6<
    F1: Future + Unpin,
    F2: Future + Unpin,
    F3: Future + Unpin,
    F4: Future + Unpin,
    F5: Future + Unpin,
    F6: Future + Unpin,
>(
    f1: F1,
    f2: F2,
    f3: F3,
    f4: F4,
    f5: F5,
    f6: F6,
) -> Result<(
    SelectResult<F1>,
    SelectResult<F2>,
    SelectResult<F3>,
    SelectResult<F4>,
    SelectResult<F5>,
    SelectResult<F6>,
)> {
    run_executor(select::Select6::new(f1, f2, f3, f4, f5, f6))
}

// Combination helpers to run until all futures are complete.

/// Creates an executor that runs the two given futures to completion, returning a tuple of the
/// outputs each yields.
///
///  # Example
///
///    ```
///    use cros_async::{Executor, complete2};
///    use futures::pin_mut;
///
///    let first = async {5};
///    let second = async {6};
///    pin_mut!(first);
///    pin_mut!(second);
///    assert_eq!(complete2(first, second).unwrap_or((0,0)), (5,6));
///    ```
pub fn complete2<F1: Future + Unpin, F2: Future + Unpin>(
    f1: F1,
    f2: F2,
) -> Result<(F1::Output, F2::Output)> {
    run_executor(complete::Complete2::new(f1, f2))
}

/// Creates an executor that runs the three given futures to completion, returning a tuple of the
/// outputs each yields.
///
///  # Example
///
///    ```
///    use cros_async::{Executor, complete3};
///    use futures::pin_mut;
///
///    let first = async {5};
///    let second = async {6};
///    let third = async {7};
///    pin_mut!(first);
///    pin_mut!(second);
///    pin_mut!(third);
///    assert_eq!(complete3(first, second, third).unwrap_or((0,0,0)), (5,6,7));
///    ```
pub fn complete3<F1: Future + Unpin, F2: Future + Unpin, F3: Future + Unpin>(
    f1: F1,
    f2: F2,
    f3: F3,
) -> Result<(F1::Output, F2::Output, F3::Output)> {
    run_executor(complete::Complete3::new(f1, f2, f3))
}

/// Creates an executor that runs the four given futures to completion, returning a tuple of the
/// outputs each yields.
///
///  # Example
///
///    ```
///    use cros_async::{Executor, complete4};
///    use futures::pin_mut;
///
///    let first = async {5};
///    let second = async {6};
///    let third = async {7};
///    let fourth = async {8};
///    pin_mut!(first);
///    pin_mut!(second);
///    pin_mut!(third);
///    pin_mut!(fourth);
///    assert_eq!(complete4(first, second, third, fourth).unwrap_or((0,0,0,0)), (5,6,7,8));
///    ```
pub fn complete4<F1: Future + Unpin, F2: Future + Unpin, F3: Future + Unpin, F4: Future + Unpin>(
    f1: F1,
    f2: F2,
    f3: F3,
    f4: F4,
) -> Result<(F1::Output, F2::Output, F3::Output, F4::Output)> {
    run_executor(complete::Complete4::new(f1, f2, f3, f4))
}

/// Creates an executor that runs the five given futures to completion, returning a tuple of the
/// outputs each yields.
///
///  # Example
///
///    ```
///    use cros_async::{Executor, complete5};
///    use futures::pin_mut;
///
///    let first = async {5};
///    let second = async {6};
///    let third = async {7};
///    let fourth = async {8};
///    let fifth = async {9};
///    pin_mut!(first);
///    pin_mut!(second);
///    pin_mut!(third);
///    pin_mut!(fourth);
///    pin_mut!(fifth);
///    assert_eq!(complete5(first, second, third, fourth, fifth).unwrap_or((0,0,0,0,0)),
///               (5,6,7,8,9));
///    ```
pub fn complete5<
    F1: Future + Unpin,
    F2: Future + Unpin,
    F3: Future + Unpin,
    F4: Future + Unpin,
    F5: Future + Unpin,
>(
    f1: F1,
    f2: F2,
    f3: F3,
    f4: F4,
    f5: F5,
) -> Result<(F1::Output, F2::Output, F3::Output, F4::Output, F5::Output)> {
    run_executor(complete::Complete5::new(f1, f2, f3, f4, f5))
}

// Functions to be used by `Future` implementations

/// Tells the waking system to wake `waker` when `fd` becomes readable.
/// The 'fd' must be fully owned by the future adding the waker, and must not be closed until the
/// next time the future is polled. If the fd is closed, there is a race where another FD can be
/// opened on top of it causing the next poll to access the new target file.
/// Returns a `WakerToken` that can be used to cancel the waker before it completes.
pub fn add_read_waker(fd: RawFd, waker: Waker) -> Result<WakerToken> {
    if use_uring() {
        uring_executor::add_read_waker(fd, waker).map_err(Error::URingExecutor)
    } else {
        fd_executor::add_read_waker(fd, waker).map_err(Error::FdExecutor)
    }
}

/// Tells the waking system to wake `waker` when `fd` becomes writable.
/// The 'fd' must be fully owned by the future adding the waker, and must not be closed until the
/// next time the future is polled. If the fd is closed, there is a race where another FD can be
/// opened on top of it causing the next poll to access the new target file.
/// Returns a `WakerToken` that can be used to cancel the waker before it completes.
pub fn add_write_waker(fd: RawFd, waker: Waker) -> Result<WakerToken> {
    if use_uring() {
        uring_executor::add_write_waker(fd, waker).map_err(Error::URingExecutor)
    } else {
        fd_executor::add_write_waker(fd, waker).map_err(Error::FdExecutor)
    }
}

/// Cancels the waker that returned the given token if the waker hasn't yet fired.
pub fn cancel_waker(token: WakerToken) -> Result<()> {
    if use_uring() {
        uring_executor::cancel_waker(token).map_err(Error::URingExecutor)
    } else {
        fd_executor::cancel_waker(token).map_err(Error::FdExecutor)
    }
}

/// Adds a new top level future to the Executor.
/// These futures must return `()`, indicating they are intended to create side-effects only.
pub fn add_future(future: Pin<Box<dyn Future<Output = ()>>>) -> Result<()> {
    if use_uring() {
        uring_executor::add_future(future).map_err(Error::URingExecutor)
    } else {
        fd_executor::add_future(future).map_err(Error::FdExecutor)
    }
}
