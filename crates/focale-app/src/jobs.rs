//! Background compute scheduler.
//!
//! One pool owns every non-UI computation. Jobs carry a priority; the UI
//! thread submits work and polls results each frame. Priorities implement
//! the docs/subsystems/preview.md contract: interactive preview beats thumbnails beats exports,
//! and AI-suggestion work runs only when everything else for the opened
//! file is idle (the v1 suggestion engine is a stub, but its scheduling is
//! wired now).

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Condvar, Mutex};

/// Job classes, highest urgency first (docs/subsystems/preview.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    /// Slider-to-screen preview updates.
    Preview,
    /// Filmstrip thumbnails.
    Thumbnail,
    /// Background export queue.
    Export,
    /// Idle-time work (AI suggestions): runs only when nothing above is
    /// queued or running.
    Idle,
}

/// A unit of background work.
struct Job {
    priority: Priority,
    seq: u64,
    /// Set when a newer job supersedes this one (e.g. a newer preview
    /// request for the same image); superseded jobs are skipped.
    cancelled: Arc<AtomicBool>,
    run: Box<dyn FnOnce() + Send>,
}

impl PartialEq for Job {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.seq == other.seq
    }
}
impl Eq for Job {}
impl PartialOrd for Job {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Job {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap is a max-heap: smaller (Priority, seq) = more urgent.
        (other.priority, other.seq).cmp(&(self.priority, self.seq))
    }
}

struct SchedulerState {
    queue: BinaryHeap<Job>,
    shutdown: bool,
    /// Jobs queued or running per non-idle class; idle work waits for zero.
    busy_non_idle: u64,
}

/// Handle used to cancel a submitted job.
#[derive(Clone)]
pub struct JobHandle {
    cancelled: Arc<AtomicBool>,
}

impl JobHandle {
    /// Marks the job as superseded; it will be skipped if not yet started.
    pub fn cancel(&self) {
        self.cancelled.store(true, AtomicOrdering::Relaxed);
    }
}

/// The background scheduler. Cloneable; owns worker threads for the life of
/// the app.
#[derive(Clone)]
pub struct Scheduler {
    inner: Arc<(Mutex<SchedulerState>, Condvar)>,
    seq: Arc<AtomicU64>,
}

impl Scheduler {
    /// Spawns `workers` threads (at least 1).
    pub fn new(workers: usize) -> Self {
        let inner = Arc::new((
            Mutex::new(SchedulerState {
                queue: BinaryHeap::new(),
                shutdown: false,
                busy_non_idle: 0,
            }),
            Condvar::new(),
        ));
        let scheduler = Self {
            inner,
            seq: Arc::new(AtomicU64::new(0)),
        };
        for _ in 0..workers.max(1) {
            let inner = scheduler.inner.clone();
            std::thread::spawn(move || worker_loop(inner));
        }
        scheduler
    }

    /// Submits a job; returns a cancellation handle.
    pub fn submit(&self, priority: Priority, run: impl FnOnce() + Send + 'static) -> JobHandle {
        let cancelled = Arc::new(AtomicBool::new(false));
        let job = Job {
            priority,
            seq: self.seq.fetch_add(1, AtomicOrdering::Relaxed),
            cancelled: cancelled.clone(),
            run: Box::new(run),
        };
        let (lock, cvar) = &*self.inner;
        {
            let mut state = lock.lock().unwrap();
            if priority != Priority::Idle {
                state.busy_non_idle += 1;
            }
            state.queue.push(job);
        }
        cvar.notify_one();
        JobHandle { cancelled }
    }

    /// Stops the worker threads.
    ///
    /// `shutdown` existed but was never written to, so the workers parked on
    /// the condvar for the life of the process. Called from
    /// `eframe::App::on_exit`. Jobs already running finish; queued jobs are
    /// dropped, which is correct for previews and thumbnails and is why
    /// `on_exit` flushes sidecars *before* calling this.
    pub fn shutdown(&self) {
        let (lock, cvar) = &*self.inner;
        {
            let mut state = lock.lock().unwrap();
            state.shutdown = true;
            state.queue.clear();
        }
        cvar.notify_all();
    }
}

fn worker_loop(inner: Arc<(Mutex<SchedulerState>, Condvar)>) {
    let (lock, cvar) = &*inner;
    loop {
        let job = {
            let mut state = lock.lock().unwrap();
            loop {
                if state.shutdown {
                    return;
                }
                // Idle jobs run only when no non-idle work is queued/running.
                let can_pop = match state.queue.peek() {
                    None => false,
                    Some(j) => j.priority != Priority::Idle || state.busy_non_idle == 0,
                };
                if can_pop {
                    break state.queue.pop().unwrap();
                }
                state = cvar.wait(state).unwrap();
            }
        };
        let skip = job.cancelled.load(AtomicOrdering::Relaxed);
        if !skip {
            (job.run)();
        }
        if job.priority != Priority::Idle {
            let mut state = lock.lock().unwrap();
            state.busy_non_idle -= 1;
            if state.busy_non_idle == 0 {
                cvar.notify_all();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    #[test]
    fn shutdown_drops_queued_work_and_stops_workers() {
        let s = Scheduler::new(1);
        let (tx, rx) = channel();
        // Hold the only worker so the jobs behind it stay queued.
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let g = gate.clone();
        s.submit(Priority::Preview, move || {
            let (l, c) = &*g;
            let mut open = l.lock().unwrap();
            while !*open {
                open = c.wait(open).unwrap();
            }
        });
        for _ in 0..4 {
            let t = tx.clone();
            s.submit(Priority::Preview, move || {
                let _ = t.send("ran");
            });
        }
        drop(tx);

        s.shutdown();
        {
            let (l, c) = &*gate;
            *l.lock().unwrap() = true;
            c.notify_all();
        }

        // The queued jobs were dropped, so nothing else reports in and the
        // channel closes once the worker exits.
        assert!(rx.recv_timeout(Duration::from_secs(2)).is_err());
    }

    #[test]
    fn priority_orders_work() {
        let s = Scheduler::new(1);
        let (tx, rx) = channel();
        // Block the single worker so ordering is decided by the heap.
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let g = gate.clone();
        s.submit(Priority::Preview, move || {
            let (l, c) = &*g;
            let mut open = l.lock().unwrap();
            while !*open {
                open = c.wait(open).unwrap();
            }
        });
        let t1 = tx.clone();
        s.submit(Priority::Export, move || {
            let _ = t1.send("export");
        });
        let t2 = tx.clone();
        s.submit(Priority::Preview, move || {
            let _ = t2.send("preview");
        });
        {
            let (l, c) = &*gate;
            *l.lock().unwrap() = true;
            c.notify_all();
        }
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "preview");
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "export");
    }

    #[test]
    fn idle_waits_for_quiet() {
        let s = Scheduler::new(2);
        let (tx, rx) = channel();

        // The non-idle job must be in flight *before* the idle job is
        // submitted, otherwise a worker may legitimately run the idle job
        // while the queue is genuinely quiet — which is the scheduler
        // behaving correctly, not the invariant under test. Gating on a
        // condvar rather than sleeping makes that ordering guaranteed instead
        // of merely likely.
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let g = gate.clone();
        let t_thumb = tx.clone();
        s.submit(Priority::Thumbnail, move || {
            let (l, c) = &*g;
            let mut open = l.lock().unwrap();
            while !*open {
                open = c.wait(open).unwrap();
            }
            drop(open);
            let _ = t_thumb.send("thumb");
        });

        // Wait until the thumbnail job is actually running, so `busy_non_idle`
        // is non-zero no matter how the workers were scheduled.
        while s.inner.0.lock().unwrap().busy_non_idle == 0 {
            std::thread::yield_now();
        }

        let t_idle = tx.clone();
        s.submit(Priority::Idle, move || {
            let _ = t_idle.send("idle");
        });

        // A worker is free, but idle work must still wait for the thumbnail.
        assert!(
            rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "idle work ran while a non-idle job was in flight"
        );

        {
            let (l, c) = &*gate;
            *l.lock().unwrap() = true;
            c.notify_all();
        }
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "thumb");
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "idle");
    }

    #[test]
    fn cancelled_jobs_are_skipped() {
        let s = Scheduler::new(1);
        let (tx, rx) = channel();
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let g = gate.clone();
        s.submit(Priority::Preview, move || {
            let (l, c) = &*g;
            let mut open = l.lock().unwrap();
            while !*open {
                open = c.wait(open).unwrap();
            }
        });
        let t = tx.clone();
        let handle = s.submit(Priority::Preview, move || {
            let _ = t.send("should-not-run");
        });
        handle.cancel();
        let t2 = tx.clone();
        s.submit(Priority::Preview, move || {
            let _ = t2.send("runs");
        });
        {
            let (l, c) = &*gate;
            *l.lock().unwrap() = true;
            c.notify_all();
        }
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "runs");
    }
}
