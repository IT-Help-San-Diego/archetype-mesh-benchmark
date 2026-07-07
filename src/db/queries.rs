use sqlx::PgPool;
use crate::models::benchmark::BenchmarkRow;
use crate::models::model_entry::ModelEntry;
use crate::error::AppResult;

pub async fn fetch_all_benchmarks(db: &PgPool) -> AppResult<Vec<BenchmarkRow>> {
    let rows = sqlx::query_as::<_, BenchmarkRow>(
        r#"SELECT model, provider, test, verdict, detail, date FROM legacy_matrix ORDER BY date DESC"#
    )
    .fetch_all(db)
    .await?;
    Ok(rows)
}

pub async fn fetch_unique_models(db: &PgPool) -> AppResult<Vec<ModelEntry>> {
    let rows = sqlx::query_as::<_, ModelEntry>(
        r#"SELECT DISTINCT model as key, model as name, provider, test as kind, 0::int4 as vision, 0::int4 as tools, model as local_path FROM legacy_matrix LIMIT 50"#
    )
    .fetch_all(db)
    .await?;
    Ok(rows)
}
