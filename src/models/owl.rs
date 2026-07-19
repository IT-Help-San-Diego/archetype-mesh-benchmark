//! Owl Semaphore data types.
//!
//! `MetacognitiveScore` is the σₕ record — one row per trial, evaluating
//! the explanation a model already gave (`trial_results.reasoning_content`,
//! migration 018), never a new question and never a second model grading
//! the first. See migrations/036_owl_semaphore.sql for the full four-owl
//! taxonomy (I / N / C / M) that `tests.owl_type` carries, and its mapping
//! onto the Klein four-group V4 = {I, σᵥ, C2, σₕ}.
use serde::{Deserialize, Serialize};

// NOTE: these two types are the data models for the Owl Semaphore metacognitive
// (σₕ) pass and the family-coverage roll-up. They are not yet constructed in the
// happy path (the metacognitive scoring is wired but the persist path is staged),
// so clippy flags them as dead. Kept intentionally — removing them would break the
// migration 018 / 036 contract. Allowed rather than deleted.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MetacognitiveScore {
    pub id: i32,
    pub trial_result_id: i32,
    /// Does the reasoning trace name the rule this test actually tests?
    /// Deterministic keyword match — see scoring::score_metacognition.
    /// NULL = no reasoning_content was present to check.
    pub cites_correct_rule: Option<bool>,
    /// Reserved — not yet scored. See scoring::score_metacognition.
    pub acknowledges_uncertainty: Option<bool>,
    /// Reserved — not yet scored. See scoring::score_metacognition.
    pub explains_distractor: Option<bool>,
    pub rubric_notes: Option<String>,
}

/// Result of the σₕ pass, before it's persisted. Field-for-field the same
/// shape as `MetacognitiveScore` minus the two DB-assigned ids.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetacognitiveResult {
    pub cites_correct_rule: Option<bool>,
    pub acknowledges_uncertainty: Option<bool>,
    pub explains_distractor: Option<bool>,
    pub rubric_notes: Option<String>,
}

/// Roll-up for the Test Registry UI / an `/api/owl/coverage` route: for
/// each Identity test, does it have paraphrase (N) and/or adversarial (C)
/// siblings yet? Mirrors the `owl_family_coverage` SQL view 1:1 — query
/// the view directly rather than recomputing this in Rust.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct OwlFamilyCoverage {
    pub identity_id: i32,
    pub identity_name: String,
    pub axis: String,
    pub formal_spec: Option<String>,
    pub non_normative_count: i64,
    pub critical_count: i64,
    pub fully_instrumented: bool,
}
