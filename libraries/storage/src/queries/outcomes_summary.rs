use chrono::{DateTime, Utc};
use eyre::WrapErr;
use sqlx::PgPool;

#[derive(Clone, Debug)]
pub struct OutcomeSummary {
    pub horizon: String,
    pub count: i64,
    pub hits: i64,
    pub hit_rate: Option<f64>,
    pub avg_r_real: Option<f64>,
    pub avg_r_predicted: Option<f64>,
    pub avg_time_in_range: Option<f64>,
}

// One row per horizon (24h / 72h / 14d) over the finalisations in [since, until) -- the
// evidence-base scorecard, and the only table this crate never lets expire.
pub async fn outcomes_summary(
    pool: &PgPool,
    since: DateTime<Utc>,
    until: DateTime<Utc>,
) -> eyre::Result<Vec<OutcomeSummary>> {
    let rows = sqlx::query_as!(
        OutcomeSummary,
        r#"
        SELECT
            horizon,
            count(*) AS "count!",
            count(*) FILTER (WHERE hit) AS "hits!",
            avg(CASE WHEN hit THEN 1.0 ELSE 0.0 END)::float8 AS hit_rate,
            avg(r_real) AS avg_r_real,
            avg(r_predicted) AS avg_r_predicted,
            avg(time_in_range) AS avg_time_in_range
        FROM outcomes
        WHERE finalized_at >= $1 AND finalized_at < $2
        GROUP BY horizon
        ORDER BY horizon
        "#,
        since,
        until,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| "Summarising outcomes")?;

    Ok(rows)
}
