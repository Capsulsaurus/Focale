//! AI-suggestion hook (architecture.md §11): v1 ships the scheduling and UI affordance
//! with a stub engine; the model arrives in v2.
//!
//! Contract implemented now: suggestions compute lazily once all other work
//! for the opened file is idle (the scheduler's `Priority::Idle` class), or
//! immediately on demand, and surface as accept / tweak / ignore proposals
//! bound to individual sliders.

use std::path::Path;

use focale_core::params::EditState;

/// One proposed parameter value.
#[derive(Debug, Clone)]
pub struct Suggestion {
    /// Human-readable target, e.g. "Tone → Exposure".
    pub label: String,
    /// Proposed new value (the UI shows current → proposed).
    pub value: f32,
    /// Applies the proposal to an edit state.
    pub apply: fn(&mut EditState, f32),
}

/// Result of a suggestion computation for one image.
#[derive(Debug, Clone, Default)]
pub struct SuggestionSet {
    /// Proposals, possibly empty.
    pub suggestions: Vec<Suggestion>,
    /// True once the engine has run for the current edit state.
    pub computed: bool,
}

/// The v1 stub engine: runs where the v2 model will run, returns no
/// proposals. Signature and call site are the v2 seam.
pub fn compute(_path: &Path, _edit: &EditState) -> SuggestionSet {
    SuggestionSet {
        suggestions: Vec::new(),
        computed: true,
    }
}

/// Per-suggestion user verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Apply the proposed value.
    Accept,
    /// Apply, then focus the slider for manual refinement.
    Tweak,
    /// Dismiss.
    Ignore,
}
