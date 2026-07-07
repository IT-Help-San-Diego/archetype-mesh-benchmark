use serde::{Deserialize, Serialize};

/// A test definition loaded from the `tests` table.
/// `expected_result` is server-side ground truth — NEVER included in the
/// prompt sent to the model (anti-cheat invariant).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TestDef {
    pub id: i32,
    pub name: String,
    pub axis: String,
    pub prompt_text: String,
    pub attachment_path: Option<String>,
    pub attachment_sha3: Option<String>,
    pub expected_result: Option<String>,
    pub scoring_method: String,
    pub trials_per_run: Option<i32>,
}
