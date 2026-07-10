use sqlx::PgPool;

use crate::error::AppResult;
use crate::models::benchmark::BenchmarkRow;
use crate::models::model_entry::ModelEntry;

/// Legacy 61-row capability matrix (historical baseline, read-only).
pub async fn fetch_all_benchmarks(db: &PgPool) -> AppResult<Vec<BenchmarkRow>> {
    let rows = sqlx::query_as::<_, BenchmarkRow>(
        r#"SELECT model, provider, test, verdict, detail, date FROM legacy_matrix ORDER BY date DESC"#,
    )
    .fetch_all(db)
    .await?;
    Ok(rows)
}

/// Model registry with per-axis verdict roll-up.
///
/// Verdict semantics (computed from the LATEST completed run per (model, axis)):
///   security axis:   SAFE (all trials passed) | UNSAFE (zero) | INTERMITTENT (partial)
///   capability axes: PASS | FAIL | INTERMITTENT — lean language: a model that can't
///   see isn't "unsafe", it just fails the capability.
/// Each axis entry also carries `ms` = average response latency across the
/// run's trials (errors excluded) — speed is a first-class measurement.
/// `verdicts` serializes as e.g. {"vision":{"v":"PASS","ms":3804},"security":{"v":"SAFE","ms":2100}}.
pub async fn fetch_unique_models(db: &PgPool) -> AppResult<Vec<ModelEntry>> {
    let rows = sqlx::query_as::<_, ModelEntry>(
        r#"
        SELECT
            m.id, m.key, m.display_name, m.provider, m.location,
            m.context_length, m.size_gb, m.notes, m.tags, m.active,
            m.created_at, m.updated_at,
            COALESCE(v.verdicts::text, '{}') AS verdicts
        FROM models m
        LEFT JOIN (
            SELECT model_id,
                   jsonb_object_agg(axis, jsonb_build_object('v', verdict, 'ms', avg_ms)) AS verdicts
            FROM (
                SELECT DISTINCT ON (r.model_id, r.axis)
                    r.model_id,
                    r.axis,
                    -- Verdict vocabulary mirrors models::verdict::compute().
                    -- Partial pass = INTERMITTENT (IEEE reliability term);
                    -- 'FLAKY' was the pre-2026-07-09 spelling.
                    CASE
                        WHEN r.axis = 'security' THEN
                            CASE WHEN r.pass_count = r.total_count THEN 'SAFE'
                                 WHEN r.pass_count = 0 THEN 'UNSAFE'
                                 ELSE 'INTERMITTENT' END
                        ELSE
                            CASE WHEN r.pass_count = r.total_count THEN 'PASS'
                                 WHEN r.pass_count = 0 THEN 'FAIL'
                                 ELSE 'INTERMITTENT' END
                    END AS verdict,
                    (SELECT ROUND(AVG(t.latency_ms))::bigint
                     FROM trial_results t
                     WHERE t.run_id = r.id AND t.latency_ms >= 0) AS avg_ms
                FROM test_runs r
                WHERE r.status = 'done' AND r.total_count > 0
                ORDER BY r.model_id, r.axis, r.created_at DESC
            ) latest
            GROUP BY model_id
        ) v ON v.model_id = m.id
        WHERE m.active = true
        ORDER BY m.display_name
        "#,
    )
    .fetch_all(db)
    .await?;
    Ok(rows)
}
