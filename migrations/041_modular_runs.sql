-- v041: modular-segment runs.
-- Adds test_ids jsonb to test_runs so a 'custom' run can execute an explicit
-- subset of tests (across any axes) instead of a whole axis. Supports the
-- "run only the problem areas" workflow: pick the LOGIC-* family, or the
-- tests that failed in the last run, and re-measure just those.
ALTER TABLE test_runs ADD COLUMN IF NOT EXISTS test_ids JSONB;

-- Index for auditing which runs were modular vs whole-axis.
CREATE INDEX IF NOT EXISTS idx_test_runs_custom ON test_runs(axis) WHERE axis = 'custom';
