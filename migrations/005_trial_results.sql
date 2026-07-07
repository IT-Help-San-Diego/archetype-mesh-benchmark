-- v005: Per-trial evidence + provenance column
CREATE TABLE IF NOT EXISTS trial_results (
    id SERIAL PRIMARY KEY,
    run_id INT NOT NULL REFERENCES test_runs(id) ON DELETE CASCADE,
    trial_num INT NOT NULL,
    raw_response TEXT,
    latency_ms BIGINT,
    passed BOOLEAN NOT NULL,
    detail TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_trial_results_run ON trial_results(run_id);

-- SHA-3 provenance of the full evidence record (immutable audit trail)
ALTER TABLE test_runs ADD COLUMN IF NOT EXISTS sha3_provenance TEXT;
