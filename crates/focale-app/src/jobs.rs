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
        let t1 = tx.clone();
        s.submit(Priority::Idle, move || {
            let _ = t1.send("idle");
        });
        let t2 = tx.clone();
        s.submit(Priority::Thumbnail, move || {
            std::thread::sleep(Duration::from_millis(50));
            let _ = t2.send("thumb");
        });
        // Idle must not run before the thumbnail finishes even with a free
        // worker available.
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
