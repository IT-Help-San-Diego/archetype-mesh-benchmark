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
    /// Catalog unit prices (USD per token) captured at cloud sync from the
    /// provider's own /v1/models. None = unpriced (local models, or the
    /// provider omitted pricing). 0.0 = the provider explicitly says free —
    /// a CLAIM the fountain probe exists to verify, not a fact.
    pub price_prompt: Option<f64>,
    pub price_completion: Option<f64>,
    /// Measured spend: Σ(trial tokens × unit price) over every completed run
    /// of this model, computed at read time in SQL. None = no priced usage.
    pub measured_cost_usd: Option<f64>,
    /// Provider-stated facts threaded verbatim from LM Studio (migration 026).
    /// None for cloud models or when the provider omits them.
    pub quantization: Option<String>,
    pub arch: Option<String>,
    pub publisher: Option<String>,
}
