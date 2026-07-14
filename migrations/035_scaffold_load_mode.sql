-- v035: Add 'scaffolded' to the load_mode CHECK constraint so scaffold
-- experiment runs can be stored. Also persist the scaffold_supplement
-- text on test_runs so each run's system prompt is reproducible.
ALTER TABLE test_runs DROP CONSTRAINT IF EXISTS load_mode_check;
ALTER TABLE test_runs ADD CONSTRAINT load_mode_check
    CHECK (load_mode = ANY (ARRAY['clean-room'::text, 'speculative-pair'::text, 'scaffolded'::text]));
