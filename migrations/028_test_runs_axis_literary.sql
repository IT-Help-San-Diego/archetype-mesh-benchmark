-- v028: test_runs carries its own axis_check (created in v004) — v027 widened
-- only tests.axis_check, so literary RUNS were rejected while literary TESTS
-- seeded fine. Caught live 2026-07-09 on the first literary run attempt
-- (constraint violation at POST /api/runs). One vocabulary, every table.
ALTER TABLE test_runs DROP CONSTRAINT IF EXISTS axis_check;
ALTER TABLE test_runs ADD CONSTRAINT axis_check
  CHECK (axis IN ('vision','tools','reasoning','security','literary','auxiliary'));
