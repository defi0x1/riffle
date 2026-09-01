use chrono::{DateTime, Utc};
use eyre::WrapErr;
use sqlx::PgPool;

use crate::types::Timeframe;

// Feeds `engine::triggers::HistoryPoint` directly -- the exit persistence windows are a
// lookback over indicators already on disk, not a stateful accumulator, so this is the only
// state the signal worker needs to reconstruct them after a restart. Ascending by bucket_start,
// matching what `engine::triggers::evaluate` expects.
#[derive(Clone, Debug)]
pub struct IndicatorHistoryPoint {
    pub bucket_start: DateTime<Utc>,
    pub r_org: Option<f64>,
    pub vol_tvl: Option<f64>,
}

async fn indicator_history_5m(
    pool: &PgPool,
    pool_address: &str,
    since: DateTime<Utc>,
) -> eyre::Result<Vec<IndicatorHistoryPoint>> {
    let rows = sqlx::query_as!(
        IndicatorHistoryPoint,
        r#"
        SELECT bucket_start, r_org, vol_tvl
        FROM indicators_5m
        WHERE pool_address = $1 AND bucket_start >= $2
        ORDER BY bucket_start ASC
        "#,
        pool_address,
        since,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| format!("Querying indicators_5m history for {pool_address}"))?;

    Ok(rows)
}

async fn indicator_history_10m(
    pool: &PgPool,
    pool_address: &str,
    since: DateTime<Utc>,
) -> eyre::Result<Vec<IndicatorHistoryPoint>> {
    let rows = sqlx::query_as!(
        IndicatorHistoryPoint,
        r#"
        SELECT bucket_start, r_org, vol_tvl
        FROM indicators_10m
        WHERE pool_address = $1 AND bucket_start >= $2
        ORDER BY bucket_start ASC
        "#,
        pool_address,
        since,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| format!("Querying indicators_10m history for {pool_address}"))?;

    Ok(rows)
}

async fn indicator_history_1h(
    pool: &PgPool,
    pool_address: &str,
    since: DateTime<Utc>,
) -> eyre::Result<Vec<IndicatorHistoryPoint>> {
    let rows = sqlx::query_as!(
        IndicatorHistoryPoint,
        r#"
        SELECT bucket_start, r_org, vol_tvl
        FROM indicators_1h
        WHERE pool_address = $1 AND bucket_start >= $2
        ORDER BY bucket_start ASC
        "#,
        pool_address,
        since,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| format!("Querying indicators_1h history for {pool_address}"))?;

    Ok(rows)
}

async fn indicator_history_4h(
    pool: &PgPool,
    pool_address: &str,
    since: DateTime<Utc>,
) -> eyre::Result<Vec<IndicatorHistoryPoint>> {
    let rows = sqlx::query_as!(
        IndicatorHistoryPoint,
        r#"
        SELECT bucket_start, r_org, vol_tvl
        FROM indicators_4h
        WHERE pool_address = $1 AND bucket_start >= $2
        ORDER BY bucket_start ASC
        "#,
        pool_address,
        since,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| format!("Querying indicators_4h history for {pool_address}"))?;

    Ok(rows)
}

async fn indicator_history_24h(
    pool: &PgPool,
    pool_address: &str,
    since: DateTime<Utc>,
) -> eyre::Result<Vec<IndicatorHistoryPoint>> {
    let rows = sqlx::query_as!(
        IndicatorHistoryPoint,
        r#"
        SELECT bucket_start, r_org, vol_tvl
        FROM indicators_24h
        WHERE pool_address = $1 AND bucket_start >= $2
        ORDER BY bucket_start ASC
        "#,
        pool_address,
        since,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| format!("Querying indicators_24h history for {pool_address}"))?;

    Ok(rows)
}

pub async fn indicator_history(
    pool: &PgPool,
    timeframe: Timeframe,
    pool_address: &str,
    since: DateTime<Utc>,
) -> eyre::Result<Vec<IndicatorHistoryPoint>> {
    match timeframe {
        Timeframe::M5 => indicator_history_5m(pool, pool_address, since).await,
        Timeframe::M10 => indicator_history_10m(pool, pool_address, since).await,
        Timeframe::H1 => indicator_history_1h(pool, pool_address, since).await,
        Timeframe::H4 => indicator_history_4h(pool, pool_address, since).await,
        Timeframe::H24 => indicator_history_24h(pool, pool_address, since).await,
    }
}
