// SPDX-License-Identifier: LGPL-2.1-or-later
//! A pool of threads kept between reads, so that a read split into bands starts no thread and
//! allocates nothing.
//!
//! `Pool::run(count, job)` calls `job(0)` on the calling thread and `job(1)` to `job(count - 1)`
//! on the pool's threads, and returns when every call has returned. Because it waits, `job` may
//! borrow from the caller's stack, as a scoped thread may: the pool holds a pointer to it only
//! while `run` is running, and `run` takes the pool by `&mut`, so no two rounds overlap. Waking
//! the threads and waiting for them use a mutex and two condition variables, which allocate
//! nothing. Both sides spin for up to `SPIN` before they sleep, because waking a sleeping thread
//! costs tens of microseconds, as much as a small read.

use std::any::Any;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// A job: called once for each index from 1 to `count - 1` on the pool's threads.
type Job = dyn Fn(usize) + Sync;

/// How long a thread looks for its next round, or the caller for the end of one, before it
/// sleeps. A tuning value: long enough to bridge reads made one after another, short enough that
/// an idle pool soon gives its cores back.
const SPIN: Duration = Duration::from_micros(50);

/// What the threads share.
struct Shared {
    state: Mutex<State>,
    /// The current round, as `State::round`, read without the lock while spinning. A hint only:
    /// every decision is made again under the lock.
    posted: AtomicU64,
    /// As `State::remaining`, read without the lock while spinning; a hint as `posted` is.
    left: AtomicUsize,
    /// Signaled when a job is posted, or when the pool is dropped.
    start: Condvar,
    /// Signaled when a thread finishes its part of a job.
    done: Condvar,
}

impl Shared {
    /// The state, whether or not a thread panicked while holding it: no code that can panic runs
    /// under the lock between changes that must go together.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct State {
    /// The job of the current round, valid only while `run` waits for it.
    job: Option<JobRef>,
    /// Counts the rounds, so that a thread runs each job once.
    round: u64,
    /// Indexes of the current round not yet finished, the calling thread's excluded.
    remaining: usize,
    /// Indexes of the current round, the calling thread's included.
    count: usize,
    /// The first panic of a part of the current round on a pool thread, to be raised by `run`.
    panic: Option<Box<dyn Any + Send>>,
    /// The pool is being dropped.
    stop: bool,
}

/// A pointer to the job of the current round, with its lifetime erased.
#[derive(Clone, Copy)]
struct JobRef(*const Job);

// The pointer is followed only while `Pool::run` waits for the round it belongs to, so the job,
// which is `Sync`, outlives every use, on whichever thread.
unsafe impl Send for JobRef {}

/// Threads kept between reads.
pub struct Pool {
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
}

impl Pool {
    /// Starts `size` threads, so that a round can run `size + 1` parts at once with the calling
    /// thread's.
    pub fn new(size: usize) -> Pool {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                job: None,
                round: 0,
                remaining: 0,
                count: 0,
                panic: None,
                stop: false,
            }),
            posted: AtomicU64::new(0),
            left: AtomicUsize::new(0),
            start: Condvar::new(),
            done: Condvar::new(),
        });
        // The pool exists before its first thread, so that if starting a thread fails, dropping
        // the pool stops and joins those already started.
        let mut pool = Pool {
            shared,
            threads: Vec::with_capacity(size),
        };
        for t in 0..size {
            let shared = Arc::clone(&pool.shared);
            pool.threads
                .push(std::thread::spawn(move || worker(&shared, t + 1)));
        }
        pool
    }

    /// How many parts a round can run at once, the calling thread's included.
    pub fn width(&self) -> usize {
        self.threads.len() + 1
    }

    /// Calls `job(i)` for each `i` below `count`: 0 on this thread, the rest on the pool's, and
    /// returns when all have returned. A part that panics makes `run` raise its panic once every
    /// part has ended, and the pool can be used again.
    pub fn run(&mut self, count: usize, job: &(dyn Fn(usize) + Sync)) {
        assert!(
            count <= self.width(),
            "a round of {count} parts, for a pool of {}",
            self.width()
        );
        if count <= 1 {
            if count == 1 {
                job(0);
            }
            return;
        }
        // The lifetime is erased here and restored by waiting below: no thread follows the
        // pointer after `remaining` reaches 0, and `run` does not return before that.
        let erased: *const Job =
            unsafe { std::mem::transmute::<*const (dyn Fn(usize) + Sync + '_), *const Job>(job) };
        {
            let mut state = self.shared.lock();
            state.job = Some(JobRef(erased));
            state.round += 1;
            state.count = count;
            state.remaining = count - 1;
            state.panic = None;
            self.shared.left.store(count - 1, Ordering::Release);
            self.shared.posted.store(state.round, Ordering::Release);
        }
        self.shared.start.notify_all();
        let mine = catch_unwind(AssertUnwindSafe(|| job(0)));
        let begun = Instant::now();
        while self.shared.left.load(Ordering::Acquire) > 0 && begun.elapsed() < SPIN {
            std::hint::spin_loop();
        }
        let mut state = self.shared.lock();
        while state.remaining > 0 {
            state = self
                .shared
                .done
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        state.job = None;
        let theirs = state.panic.take();
        drop(state);
        if let Err(e) = mine {
            // One panic is raised; the other's payload is forgotten rather than dropped while
            // unwinding, where a panic in its drop would abort.
            std::mem::forget(theirs);
            resume_unwind(e);
        }
        if let Some(e) = theirs {
            resume_unwind(e);
        }
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        {
            let mut state = self.shared.lock();
            state.stop = true;
            // A spinning thread looks at the round first.
            state.round += 1;
            self.shared.posted.store(state.round, Ordering::Release);
        }
        self.shared.start.notify_all();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

/// A pool thread: waits for each round, runs its part if the round has one for it, and reports.
fn worker(shared: &Shared, index: usize) {
    let mut seen = 0;
    loop {
        let begun = Instant::now();
        while shared.posted.load(Ordering::Acquire) == seen && begun.elapsed() < SPIN {
            std::hint::spin_loop();
        }
        let job = {
            let mut state = shared.lock();
            while !state.stop && state.round == seen {
                state = shared
                    .start
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            if state.stop {
                return;
            }
            seen = state.round;
            if index >= state.count {
                // This round has no part for this thread.
                continue;
            }
            // A round with a part for this thread holds its job until the part has ended.
            state.job.expect("a round with parts has a job")
        };
        // `run` waits for this part before the job it points to can go. A panic's payload is
        // kept, not dropped here, so that dropping it cannot escape and leave the round open.
        let outcome = catch_unwind(AssertUnwindSafe(|| unsafe { (*job.0)(index) }));
        let mut state = shared.lock();
        if let Err(payload) = outcome {
            if state.panic.is_none() {
                state.panic = Some(payload);
            } else {
                // A second panic in one round: its payload is not needed, and dropping it could
                // panic in turn, so it is forgotten.
                std::mem::forget(payload);
            }
        }
        state.remaining -= 1;
        shared.left.store(state.remaining, Ordering::Release);
        if state.remaining == 0 {
            shared.done.notify_all();
        }
    }
}
