-- Quarantine answer-leak-contaminated scaffolded runs.
--
-- Before the companion code fix, scaffolded runs (load_mode = 'scaffolded')
-- with a non-empty scaffold_supplement had each test's formal_spec appended to
-- the model's system prompt. The formal_spec IS the ground-truth answer (⊢/⊬ =
-- VALID/INVALID; vision specs name the literal expected string; security specs
-- state the required refusal), so those runs were handed an answer key — their
-- scores are not honest capability measurements and must not appear in any
-- leaderboard/aggregate.
--
-- Every sealed scaffolded run predating the fix is contaminated (the leak was
-- unconditional whenever scaffold_supplement was non-empty, and the pair route
-- requires a non-empty supplement). Quarantine them with a distinct reason so
-- they are excluded from aggregates yet preserved for audit. The reason value
-- is admitted by the constraint extended in migration 045. Idempotent.
UPDATE test_runs
SET quarantined = TRUE,
    quarantine_reason = 'answer_leak_contamination'
WHERE load_mode = 'scaffolded'
  AND status = 'done'
  AND (quarantined IS NULL OR quarantined = FALSE);
