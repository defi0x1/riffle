use chrono::{DateTime, Utc};
use eyre::WrapErr;
use rust_decimal::Decimal;
use sqlx::PgPool;

use crate::types::Timeframe;

// A row for /volume: ranked by raw volume_usd (from pool_metrics_{tf}), with the
// bucket-over-bucket vol_change that indicators_{tf} already computed at write time riding
// along in the same row -- the caller no longer pays a per-row detail query for it.
#[derive(Clone, Debug)]
pub struct VolumeRanking {
    pub pool_address: String,
    pub venue: i16,
    pub token_x: String,
    pub token_y: String,
    pub tvl_usd: Option<Decimal>,
    pub tier: i16,
    pub bucket_start: DateTime<Utc>,
    pub quality: String,
    pub volume_usd: Option<Decimal>,
    pub vol_change: Option<f64>,
}

async fn volume_ranked_pools_5m(
    pool: &PgPool,
    venue: i16,
    limit: i64,
) -> eyre::Result<Vec<VolumeRanking>> {
    let rows = sqlx::query_as!(
        VolumeRanking,
        r#"
        SELECT
            p.pool_address, p.venue, p.token_x, p.token_y, p.tvl_usd, p.tier,
            m.bucket_start as "bucket_start!", i.quality, m.volume_usd, i.vol_change
        FROM pool_metrics_5m m
        JOIN pools p ON p.pool_address = m.pool_address
        JOIN indicators_5m i ON i.pool_address = m.pool_address AND i.bucket_start = m.bucket_start
        WHERE p.venue = $1
          AND m.bucket_start = (SELECT max(bucket_start) FROM pool_metrics_5m)
        ORDER BY m.volume_usd DESC NULLS LAST
        LIMIT $2
        "#,
        venue,
        limit,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| "Querying volume ranking from pool_metrics_5m")?;

    Ok(rows)
}

async fn volume_ranked_pools_10m(
    pool: &PgPool,
    venue: i16,
    limit: i64,
) -> eyre::Result<Vec<VolumeRanking>> {
    let rows = sqlx::query_as!(
        VolumeRanking,
        r#"
        SELECT
            p.pool_address, p.venue, p.token_x, p.token_y, p.tvl_usd, p.tier,
            m.bucket_start as "bucket_start!", i.quality, m.volume_usd, i.vol_change
        FROM pool_metrics_10m m
        JOIN pools p ON p.pool_address = m.pool_address
        JOIN indicators_10m i ON i.pool_address = m.pool_address AND i.bucket_start = m.bucket_start
        WHERE p.venue = $1
          AND m.bucket_start = (SELECT max(bucket_start) FROM pool_metrics_10m)
        ORDER BY m.volume_usd DESC NULLS LAST
        LIMIT $2
        "#,
        venue,
        limit,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| "Querying volume ranking from pool_metrics_10m")?;

    Ok(rows)
}

async fn volume_ranked_pools_1h(
    pool: &PgPool,
    venue: i16,
    limit: i64,
) -> eyre::Result<Vec<VolumeRanking>> {
    let rows = sqlx::query_as!(
        VolumeRanking,
        r#"
        SELECT
            p.pool_address, p.venue, p.token_x, p.token_y, p.tvl_usd, p.tier,
            m.bucket_start as "bucket_start!", i.quality, m.volume_usd, i.vol_change
        FROM pool_metrics_1h m
        JOIN pools p ON p.pool_address = m.pool_address
        JOIN indicators_1h i ON i.pool_address = m.pool_address AND i.bucket_start = m.bucket_start
        WHERE p.venue = $1
          AND m.bucket_start = (SELECT max(bucket_start) FROM pool_metrics_1h)
        ORDER BY m.volume_usd DESC NULLS LAST
        LIMIT $2
        "#,
        venue,
        limit,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| "Querying volume ranking from pool_metrics_1h")?;

    Ok(rows)
}

async fn volume_ranked_pools_4h(
    pool: &PgPool,
    venue: i16,
    limit: i64,
) -> eyre::Result<Vec<VolumeRanking>> {
    let rows = sqlx::query_as!(
        VolumeRanking,
        r#"
        SELECT
            p.pool_address, p.venue, p.token_x, p.token_y, p.tvl_usd, p.tier,
            m.bucket_start as "bucket_start!", i.quality, m.volume_usd, i.vol_change
        FROM pool_metrics_4h m
        JOIN pools p ON p.pool_address = m.pool_address
        JOIN indicators_4h i ON i.pool_address = m.pool_address AND i.bucket_start = m.bucket_start
        WHERE p.venue = $1
          AND m.bucket_start = (SELECT max(bucket_start) FROM pool_metrics_4h)
        ORDER BY m.volume_usd DESC NULLS LAST
        LIMIT $2
        "#,
        venue,
        limit,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| "Querying volume ranking from pool_metrics_4h")?;

    Ok(rows)
}

async fn volume_ranked_pools_24h(
    pool: &PgPool,
    venue: i16,
    limit: i64,
) -> eyre::Result<Vec<VolumeRanking>> {
    let rows = sqlx::query_as!(
        VolumeRanking,
        r#"
        SELECT
            p.pool_address, p.venue, p.token_x, p.token_y, p.tvl_usd, p.tier,
            m.bucket_start as "bucket_start!", i.quality, m.volume_usd, i.vol_change
        FROM pool_metrics_24h m
        JOIN pools p ON p.pool_address = m.pool_address
        JOIN indicators_24h i ON i.pool_address = m.pool_address AND i.bucket_start = m.bucket_start
        WHERE p.venue = $1
          AND m.bucket_start = (SELECT max(bucket_start) FROM pool_metrics_24h)
        ORDER BY m.volume_usd DESC NULLS LAST
        LIMIT $2
        "#,
        venue,
        limit,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| "Querying volume ranking from pool_metrics_24h")?;

    Ok(rows)
}

// Ranks the most recent bucket of the given timeframe by raw volume_usd -- what /volume
// actually means by "highest volume", as opposed to top_pools's r_org or vol_tvl ratio.
pub async fn volume_ranked_pools(
    pool: &PgPool,
    venue: i16,
    timeframe: Timeframe,
    limit: i64,
) -> eyre::Result<Vec<VolumeRanking>> {
    match timeframe {
        Timeframe::M5 => volume_ranked_pools_5m(pool, venue, limit).await,
        Timeframe::M10 => volume_ranked_pools_10m(pool, venue, limit).await,
        Timeframe::H1 => volume_ranked_pools_1h(pool, venue, limit).await,
        Timeframe::H4 => volume_ranked_pools_4h(pool, venue, limit).await,
        Timeframe::H24 => volume_ranked_pools_24h(pool, venue, limit).await,
    }
}
