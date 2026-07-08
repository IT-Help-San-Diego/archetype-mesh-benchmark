-- v016: Add 'aborted' as a distinct terminal status, separate from 'error'.
--
-- A user-initiated stop is not the same evidence category as a technical
-- failure (model wouldn't load, LM Studio rejected the request, budget
-- timeout). Collapsing them into 'error' would corrupt the audit trail —
-- "the operator chose to stop this" and "this broke" are different facts
-- about a run, and the dashboard/history should say which happened.
ALTER TABLE test_runs DROP CONSTRAINT IF EXISTS status_check;
ALTER TABLE test_runs ADD CONSTRAINT status_check
    CHECK (status = ANY (ARRAY['queued'::text, 'loading'::text, 'running'::text, 'done'::text, 'error'::text, 'aborted'::text]));
