-- 017: Prompt Builder history — every prompt-check run is evidence, keep it.
-- Incident 2026-07-08: a fabricated vision answer could only be recovered by
-- grepping LM Studio's server logs. The user had no way to revisit their own
-- last runs. Same evidence discipline as test_runs: persist, timestamp, hash.
CREATE TABLE prompt_history (
    id SERIAL PRIMARY KEY,
    model_key TEXT NOT NULL,
    prompt TEXT NOT NULL,
    has_image BOOLEAN NOT NULL DEFAULT FALSE,
    image_sha3 TEXT,
    response TEXT NOT NULL DEFAULT '',
    reasoning_content TEXT,
    no_final_answer BOOLEAN NOT NULL DEFAULT FALSE,
    finish_reason TEXT,
    prompt_tokens BIGINT,
    completion_tokens BIGINT,
    reasoning_tokens BIGINT,
    latency_ms BIGINT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_prompt_history_created ON prompt_history (created_at DESC);
