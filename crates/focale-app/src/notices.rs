//! User-visible notices.
//!
//! Every failure the app can hit must reach the user, not just the log
//! (`docs/subsystems/app.md` status bar). Before this existed, six of the
//! seven failure paths in the GUI were `tracing::error!`-only — including a
//! sidecar save failure, which silently lost edits. Logging stays (it is how
//! a bug report is diagnosed); this is the half the user sees.
//!
//! Notices are derived, in-memory state: they are never persisted and never
//! consulted as an authority (`[HARD-FS]`).

use std::time::{Duration, Instant};

/// How loudly a notice should read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Something worked but the user should know (e.g. a skipped stage).
    Info,
    /// Degraded, recoverable (e.g. a derived file could not be written).
    Warning,
    /// The requested operation did not happen.
    Error,
}

impl Severity {
    /// Short glyph used as the status-bar prefix.
    pub fn glyph(self) -> &'static str {
        match self {
            Severity::Info => "ℹ",
            Severity::Warning => "⚠",
            Severity::Error => "✘",
        }
    }
}

/// One thing the user should be told.
#[derive(Debug, Clone)]
pub struct Notice {
    /// How loudly it reads.
    pub severity: Severity,
    /// One line, addressed to the user rather than to a developer.
    pub text: String,
    /// When it was raised, for age-out and ordering.
    pub at: Instant,
}

/// Bounded, newest-last list of notices.
///
/// The cap exists so a failure that repeats every frame (a directory on a
/// disconnected volume, say) cannot grow without bound.
#[derive(Debug, Default)]
pub struct Notices {
    items: Vec<Notice>,
}

/// Most notices we keep; older ones are dropped oldest-first.
const CAPACITY: usize = 100;

impl Notices {
    /// Raises a notice. Consecutive duplicates are collapsed by refreshing
    /// the existing entry's timestamp instead of appending, so a repeating
    /// failure reads as one ongoing problem rather than a wall of text.
    pub fn push(&mut self, severity: Severity, text: impl Into<String>) {
        let text = text.into();
        if let Some(last) = self.items.last_mut()
            && last.severity == severity
            && last.text == text
        {
            last.at = Instant::now();
            return;
        }
        self.items.push(Notice {
            severity,
            text,
            at: Instant::now(),
        });
        if self.items.len() > CAPACITY {
            let overflow = self.items.len() - CAPACITY;
            self.items.drain(0..overflow);
        }
    }

    /// Raises an error notice.
    pub fn error(&mut self, text: impl Into<String>) {
        self.push(Severity::Error, text);
    }

    /// Raises a warning notice.
    pub fn warn(&mut self, text: impl Into<String>) {
        self.push(Severity::Warning, text);
    }

    /// Raises an informational notice.
    pub fn info(&mut self, text: impl Into<String>) {
        self.push(Severity::Info, text);
    }

    /// The most recent notice, if it is younger than `ttl`. This is what the
    /// status bar shows, so a transient failure does not sit there forever
    /// claiming to be the current state.
    pub fn current(&self, ttl: Duration) -> Option<&Notice> {
        self.items
            .last()
            .filter(|n| n.at.elapsed() < ttl || n.severity == Severity::Error)
    }

    /// All notices, oldest first.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &Notice> {
        self.items.iter()
    }

    /// How many notices are held.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether there are no notices.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Drops every notice.
    pub fn clear(&mut self) {
        self.items.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_records_in_order() {
        let mut n = Notices::default();
        n.info("first");
        n.error("second");
        let texts: Vec<&str> = n.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(texts, ["first", "second"]);
        assert_eq!(
            n.current(Duration::from_secs(5)).map(|x| x.severity),
            Some(Severity::Error)
        );
    }

    #[test]
    fn consecutive_duplicates_collapse() {
        let mut n = Notices::default();
        for _ in 0..10 {
            n.error("disk is gone");
        }
        assert_eq!(n.len(), 1);
    }

    #[test]
    fn duplicate_collapse_only_applies_to_the_newest() {
        let mut n = Notices::default();
        n.error("a");
        n.error("b");
        n.error("a");
        assert_eq!(n.len(), 3);
    }

    #[test]
    fn differing_severity_is_not_collapsed() {
        let mut n = Notices::default();
        n.warn("same text");
        n.error("same text");
        assert_eq!(n.len(), 2);
    }

    #[test]
    fn capacity_is_bounded_and_drops_oldest() {
        let mut n = Notices::default();
        for i in 0..(CAPACITY + 25) {
            n.error(format!("failure {i}"));
        }
        assert_eq!(n.len(), CAPACITY);
        // The oldest survivor is the 25th message, not the 0th.
        assert_eq!(n.iter().next().map(|x| x.text.as_str()), Some("failure 25"));
    }

    #[test]
    fn current_keeps_errors_but_ages_out_info() {
        let mut n = Notices::default();
        n.info("transient");
        assert!(n.current(Duration::from_secs(5)).is_some());
        assert!(n.current(Duration::ZERO).is_none());

        n.error("persistent");
        // Errors are not aged out: an unresolved failure stays on screen.
        assert!(n.current(Duration::ZERO).is_some());
    }

    #[test]
    fn clear_empties() {
        let mut n = Notices::default();
        n.error("x");
        assert!(!n.is_empty());
        n.clear();
        assert!(n.is_empty());
        assert!(n.current(Duration::from_secs(5)).is_none());
    }
}
