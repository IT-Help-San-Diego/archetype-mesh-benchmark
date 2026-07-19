-- v042: allow 'custom' axis on test_runs for modular-segment runs.
-- A modular run executes an explicit test_ids set (spanning any axes) as a
-- single 'custom' run, so the axis column needs the extra value.
ALTER TABLE test_runs DROP CONSTRAINT IF EXISTS axis_check;
ALTER TABLE test_runs ADD CONSTRAINT axis_check
  CHECK (axis IN ('vision','tools','reasoning','security','literary','auxiliary','custom'));
