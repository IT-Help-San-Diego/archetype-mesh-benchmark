use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct ModelEntry {
    pub id: i32,
    pub key: String,
    pub display_name: String,
    pub provider: String,
    pub location: String,
    pub context_length: i32,
    pub size_gb: f64,
    pub notes: Option<String>,
    pub tags: Option<Vec<String>>,
    pub active: bool,
    // DB columns are TIMESTAMP (without time zone) — sqlx maps those to NaiveDateTime.
    // DateTime<Utc> would require TIMESTAMPTZ and fails to decode at runtime.
    pub created_at: Option<chrono::NaiveDateTime>,
    pub updated_at: Option<chrono::NaiveDateTime>,
    /// Per-axis verdict roll-up as a JSON object string, e.g. {"vision":"SAFE","tools":"INTERMITTENT"} (legacy rows may say FLAKY; canonicalized at read time).
    /// Computed from the latest completed test_run per (model, axis); '{}' = untested.
    /// The dashboard parses this with JSON.parse(m.verdicts || '{}').
    pub verdicts: Option<String>,
}
