//! Slider-to-screen latency instrumentation (issue #11).
//!
//! `docs/subsystems/platform.md` sets a preview target of **< 100 ms** from
//! slider change to screen. This module measures it in the running app; the
//! reproducible offline benchmark is `focale-cli bench-preview`, and the
//! recorded numbers live in `docs/verification.md`.
//!
//! # What the four segments mean
//!
//! A preview update crosses two threads and one GPU submission:
//!
//! | Segment | From | To |
//! |---|---|---|
//! | `queue` | [`RenderTiming::queued`] — the slider moved and `spawn_render` submitted the job | the worker picked the job up |
//! | `pipeline` | worker start | the CPU pipeline finished |
//! | `upload` | the UI thread received the frame | the texture reached the GPU |
//! | `present` | texture uploaded | the next `update()` began |
//!
//! # Honesty note about `present`
//!
//! eframe paints *after* `update()` returns, so the app cannot observe the
//! exact moment its pixels light up. The stamp is taken at the top of the
//! **next** `update()` call, which is the first instant at which the frame
//! containing the new pixels is known to have been submitted and the
//! swapchain to have released a drawable. That interval therefore includes a
//! vsync wait — on a 60 Hz display it can add up to ~16 ms that a user does
//! not perceive as latency. **The total over-reports rather than flatters**,
//! which is the direction an honesty-first measurement should err in. Read
//! `queue + pipeline + upload` for the app's own cost and the total for the
//! wall-clock experience.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How many recent samples the overlay's median/max are computed over.
const HISTORY: usize = 60;

/// Timing collected on the worker side of one preview render.
///
/// Created when the job is submitted so the queue wait is visible; the
/// worker fills in the rest and hands it back with the frame.
#[derive(Debug, Clone, Copy)]
pub struct RenderTiming {
    /// When `spawn_render` submitted the job (i.e. when the slider moved).
    pub queued: Instant,
    /// When the worker thread began running the pipeline.
    pub started: Instant,
    /// When the pipeline returned.
    pub finished: Instant,
}

impl RenderTiming {
    /// Stamps the submission instant, to be paired with the worker's own
    /// start/finish stamps by [`Self::new`] once the render completes.
    pub fn queued_now() -> Instant {
        Instant::now()
    }

    /// Builds the timing for a completed render.
    pub fn new(queued: Instant, started: Instant, finished: Instant) -> Self {
        Self {
            queued,
            started,
            finished,
        }
    }

    fn queue_wait(&self) -> Duration {
        self.started.saturating_duration_since(self.queued)
    }

    fn pipeline(&self) -> Duration {
        self.finished.saturating_duration_since(self.started)
    }
}

/// One completed slider-to-screen measurement, in milliseconds.
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    /// Scheduler queue wait before the pipeline started.
    pub queue_ms: f32,
    /// CPU pipeline run on the preview base.
    pub pipeline_ms: f32,
    /// Working-space image → GPU texture.
    pub upload_ms: f32,
    /// Upload → the next frame beginning (includes the vsync wait; see the
    /// module docs).
    pub present_ms: f32,
    /// Slider change → that same point. The figure the budget applies to.
    pub total_ms: f32,
    /// Preview base dimensions the pipeline ran on.
    pub size: (u32, u32),
}

/// Rolling preview-latency statistics plus the debug overlay's visibility.
#[derive(Default)]
pub struct PerfStats {
    /// True while the F12 overlay is shown. Off by default: this is a debug
    /// affordance, not a status-bar field (`docs/subsystems/app.md`).
    pub overlay: bool,
    last: Option<Sample>,
    history: VecDeque<Sample>,
    /// Set when a frame has been uploaded but not yet stamped as presented.
    pending: Option<PendingPresent>,
}

/// A frame whose pixels have reached the GPU but whose present has not yet
/// been observed.
struct PendingPresent {
    timing: RenderTiming,
    uploaded: Instant,
    size: (u32, u32),
}

impl PerfStats {
    /// Records that a rendered frame's texture has just been uploaded.
    ///
    /// The sample completes on the next call to [`Self::stamp_presented`].
    pub fn frame_uploaded(&mut self, timing: RenderTiming, size: (u32, u32)) {
        self.frame_uploaded_at(timing, size, Instant::now());
    }

    /// Called at the top of every frame. Completes a pending sample, if any.
    pub fn stamp_presented(&mut self) {
        self.stamp_presented_at(Instant::now());
    }

    /// [`Self::frame_uploaded`] with an explicit clock, so the timing
    /// arithmetic is testable without sleeping.
    fn frame_uploaded_at(&mut self, timing: RenderTiming, size: (u32, u32), uploaded: Instant) {
        self.pending = Some(PendingPresent {
            timing,
            uploaded,
            size,
        });
    }

    /// [`Self::stamp_presented`] with an explicit clock.
    fn stamp_presented_at(&mut self, now: Instant) {
        let Some(p) = self.pending.take() else {
            return;
        };
        let sample = Sample {
            queue_ms: millis(p.timing.queue_wait()),
            pipeline_ms: millis(p.timing.pipeline()),
            upload_ms: millis(p.uploaded.saturating_duration_since(p.timing.finished)),
            present_ms: millis(now.saturating_duration_since(p.uploaded)),
            total_ms: millis(now.saturating_duration_since(p.timing.queued)),
            size: p.size,
        };
        tracing::debug!(
            target: "focale_app::perf",
            total_ms = sample.total_ms,
            queue_ms = sample.queue_ms,
            pipeline_ms = sample.pipeline_ms,
            upload_ms = sample.upload_ms,
            present_ms = sample.present_ms,
            width = sample.size.0,
            height = sample.size.1,
            "slider-to-screen"
        );
        self.last = Some(sample);
        self.history.push_back(sample);
        if self.history.len() > HISTORY {
            self.history.pop_front();
        }
    }

    /// The most recent completed sample.
    pub fn last(&self) -> Option<Sample> {
        self.last
    }

    /// Number of samples behind the median/max figures.
    pub fn count(&self) -> usize {
        self.history.len()
    }

    /// Median total latency over the retained history.
    pub fn median_total_ms(&self) -> Option<f32> {
        if self.history.is_empty() {
            return None;
        }
        let mut v: Vec<f32> = self.history.iter().map(|s| s.total_ms).collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        Some(v[v.len() / 2])
    }

    /// Worst total latency over the retained history.
    pub fn max_total_ms(&self) -> Option<f32> {
        self.history
            .iter()
            .map(|s| s.total_ms)
            .fold(None, |acc: Option<f32>, v| {
                Some(acc.map_or(v, |a| a.max(v)))
            })
    }
}

fn millis(d: Duration) -> f32 {
    d.as_secs_f32() * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// Records one complete sample on a fabricated clock:
    /// queue → pipeline → upload → present, each of the given duration.
    fn record(
        stats: &mut PerfStats,
        base: Instant,
        queue: u64,
        pipeline: u64,
        upload: u64,
        present: u64,
    ) {
        let started = base + ms(queue);
        let finished = started + ms(pipeline);
        let uploaded = finished + ms(upload);
        stats.frame_uploaded_at(
            RenderTiming::new(base, started, finished),
            (64, 64),
            uploaded,
        );
        stats.stamp_presented_at(uploaded + ms(present));
    }

    #[test]
    fn a_sample_completes_only_on_the_next_frame() {
        let mut stats = PerfStats::default();
        stats.stamp_presented();
        assert!(stats.last().is_none(), "nothing pending, nothing recorded");

        let base = Instant::now();
        let started = base + ms(2);
        let finished = started + ms(20);
        let uploaded = finished + ms(3);
        stats.frame_uploaded_at(
            RenderTiming::new(base, started, finished),
            (2560, 1707),
            uploaded,
        );
        assert!(stats.last().is_none(), "upload alone does not complete it");

        stats.stamp_presented_at(uploaded + ms(9));
        let s = stats.last().expect("sample recorded on the next frame");
        assert_eq!(s.size, (2560, 1707));
        assert_eq!(s.queue_ms, 2.0);
        assert_eq!(s.pipeline_ms, 20.0);
        assert_eq!(s.upload_ms, 3.0);
        assert_eq!(s.present_ms, 9.0);
        // The total spans the whole chain, so it is exactly the four parts.
        assert_eq!(s.total_ms, 34.0);
    }

    #[test]
    fn history_reports_median_and_max() {
        let mut stats = PerfStats::default();
        let base = Instant::now();
        for pipeline in [10u64, 50, 30] {
            record(&mut stats, base, 0, pipeline, 0, 0);
        }
        assert_eq!(stats.count(), 3);
        assert_eq!(stats.median_total_ms().unwrap(), 30.0);
        assert_eq!(stats.max_total_ms().unwrap(), 50.0);
    }

    #[test]
    fn history_is_bounded() {
        let mut stats = PerfStats::default();
        let base = Instant::now();
        for _ in 0..(HISTORY + 25) {
            record(&mut stats, base, 0, 1, 0, 0);
        }
        assert_eq!(stats.count(), HISTORY);
    }

    #[test]
    fn a_dropped_frame_cannot_leave_a_stale_sample_pending() {
        let mut stats = PerfStats::default();
        let base = Instant::now();
        // Two uploads in a row (a superseded frame): only the latest is
        // pending, so the completed sample describes the frame the user saw.
        let first = RenderTiming::new(base, base + ms(1), base + ms(11));
        let second = RenderTiming::new(base + ms(20), base + ms(21), base + ms(26));
        stats.frame_uploaded_at(first, (64, 64), base + ms(12));
        stats.frame_uploaded_at(second, (128, 128), base + ms(27));
        stats.stamp_presented_at(base + ms(30));
        let s = stats.last().unwrap();
        assert_eq!(s.size, (128, 128));
        assert_eq!(s.total_ms, 10.0, "measured from the second frame's queue");
        assert_eq!(stats.count(), 1);
    }
}
