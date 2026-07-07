-- Archetype Mesh Benchmark — Initial Schema (PostgreSQL)
-- This migration creates the canonical benchmark results table

CREATE TABLE IF NOT EXISTS legacy_matrix (
    id SERIAL PRIMARY KEY,
    model TEXT NOT NULL,
    provider TEXT NOT NULL,
    test TEXT NOT NULL,
    verdict TEXT NOT NULL,
    detail TEXT,
    date TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_legacy_matrix_model ON legacy_matrix(model);
CREATE INDEX IF NOT EXISTS idx_legacy_matrix_provider ON legacy_matrix(provider);
CREATE INDEX IF NOT EXISTS idx_legacy_matrix_verdict ON legacy_matrix(verdict);
CREATE INDEX IF NOT EXISTS idx_legacy_matrix_date ON legacy_matrix(date);
