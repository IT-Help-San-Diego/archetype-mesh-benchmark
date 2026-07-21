-- Reconcile historical over-quarantine.
--
-- The executor used to quarantine an ENTIRE run the moment a single trial hit
-- an infrastructure error (infra_error_count > 0), even when the surviving
-- clean trials were a perfect record. Infra trials are already excluded from
-- the capability denominator, so those runs held VALID evidence that was
-- nonetheless hidden from every aggregate (loot, router, dossier, insights) —
-- e.g. run 915, phi-4-reasoning-plus reasoning 78/78 PASS, invisible.
--
-- The new rule quarantines only when infrastructure noise DOMINATED the run
-- (more trials died than survived). This backfills that rule onto history:
-- release any run quarantined for 'infrastructure_error' whose non-infra
-- trials are at least as many as its infra trials. Genuine capability
-- failures were never quarantined under the old rule for this reason, so this
-- only ever restores wrongly-hidden valid evidence. Idempotent.
UPDATE test_runs r
SET quarantined = FALSE,
    quarantine_reason = NULL
WHERE r.quarantined = TRUE
  AND r.quarantine_reason = 'infrastructure_error'
  AND (
        SELECT COUNT(*) FILTER (WHERE tr.is_infra_error)
        FROM trial_results tr WHERE tr.run_id = r.id
      )
      <=
      (
        SELECT COUNT(*) FILTER (WHERE NOT tr.is_infra_error)
        FROM trial_results tr WHERE tr.run_id = r.id
      );
