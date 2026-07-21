-- Extend the quarantine reason whitelist (migration 032) to admit the
-- answer-leak reason used by migration 046. Kept in its own migration so the
-- constraint change is committed before any row is set to the new value.
ALTER TABLE test_runs DROP CONSTRAINT IF EXISTS quarantine_reason_check;
ALTER TABLE test_runs ADD CONSTRAINT quarantine_reason_check
    CHECK (quarantine_reason IS NULL OR quarantine_reason = ANY (
        ARRAY[
            'infrastructure_error'::text,
            'blank_responses'::text,
            'all_failed'::text,
            'timeout'::text,
            'user_excluded'::text,
            'draft_stats_mismatch'::text,
            'answer_leak_contamination'::text
        ]
    ));
