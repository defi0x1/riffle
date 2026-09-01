use chrono::{DateTime, Utc};
use eyre::WrapErr;
use rust_decimal::Decimal;
use sqlx::PgPool;

use crate::types::Timeframe;

// A row rendered by /top, /volume and a future HTTP listing -- deliberately flat (no nested pool
// + indicator structs) since every consumer here wants a table row, not a graph of objects.
#[derive(Clone, Debug)]
pub struct PoolRanking {
    pub pool_address: String,
    pub venue: i16,
    pub token_x: String,
    pub token_y: String,
    pub tvl_usd: Option<Decimal>,
    pub tier: i16,
    pub bucket_start: DateTime<Utc>,
    pub quality: String,
    pub regime: Option<String>,
    pub r_org: Option<f64>,
    pub top_score: Option<f64>,
    pub vol_tvl: Option<f64>,
    pub fee_tvl: Option<f64>,
}

async fn top_pools_5m(pool: &PgPool, venue: i16, limit: i64) -> eyre::Result<Vec<PoolRanking>> {
    let rows = sqlx::query_as!(
        PoolRanking,
        r#"
        SELECT
            p.pool_address, p.venue, p.token_x, p.token_y, p.tvl_usd, p.tier,
            i.bucket_start, i.quality, i.regime, i.r_org, i.top_score, i.vol_tvl, i.fee_tvl
        FROM indicators_5m i
        JOIN pools p ON p.pool_address = i.pool_address
        WHERE p.venue = $1
          AND i.bucket_start = (SELECT max(bucket_start) FROM indicators_5m)
        ORDER BY i.r_org DESC NULLS LAST
        LIMIT $2
        "#,
        venue,
        limit,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| "Querying top pools from indicators_5m")?;

    Ok(rows)
}

async fn top_pools_10m(pool: &PgPool, venue: i16, limit: i64) -> eyre::Result<Vec<PoolRanking>> {
    let rows = sqlx::query_as!(
        PoolRanking,
        r#"
        SELECT
            p.pool_address, p.venue, p.token_x, p.token_y, p.tvl_usd, p.tier,
            i.bucket_start, i.quality, i.regime, i.r_org, i.top_score, i.vol_tvl, i.fee_tvl
        FROM indicators_10m i
        JOIN pools p ON p.pool_address = i.pool_address
        WHERE p.venue = $1
          AND i.bucket_start = (SELECT max(bucket_start) FROM indicators_10m)
        ORDER BY i.r_org DESC NULLS LAST
        LIMIT $2
        "#,
        venue,
        limit,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| "Querying top pools from indicators_10m")?;

    Ok(rows)
}

async fn top_pools_1h(pool: &PgPool, venue: i16, limit: i64) -> eyre::Result<Vec<PoolRanking>> {
    let rows = sqlx::query_as!(
        PoolRanking,
        r#"
        SELECT
            p.pool_address, p.venue, p.token_x, p.token_y, p.tvl_usd, p.tier,
            i.bucket_start, i.quality, i.regime, i.r_org, i.top_score, i.vol_tvl, i.fee_tvl
        FROM indicators_1h i
        JOIN pools p ON p.pool_address = i.pool_address
        WHERE p.venue = $1
          AND i.bucket_start = (SELECT max(bucket_start) FROM indicators_1h)
        ORDER BY i.r_org DESC NULLS LAST
        LIMIT $2
        "#,
        venue,
        limit,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| "Querying top pools from indicators_1h")?;

    Ok(rows)
}

async fn top_pools_4h(pool: &PgPool, venue: i16, limit: i64) -> eyre::Result<Vec<PoolRanking>> {
    let rows = sqlx::query_as!(
        PoolRanking,
        r#"
        SELECT
            p.pool_address, p.venue, p.token_x, p.token_y, p.tvl_usd, p.tier,
            i.bucket_start, i.quality, i.regime, i.r_org, i.top_score, i.vol_tvl, i.fee_tvl
        FROM indicators_4h i
        JOIN pools p ON p.pool_address = i.pool_address
        WHERE p.venue = $1
          AND i.bucket_start = (SELECT max(bucket_start) FROM indicators_4h)
        ORDER BY i.r_org DESC NULLS LAST
        LIMIT $2
        "#,
        venue,
        limit,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| "Querying top pools from indicators_4h")?;

    Ok(rows)
}

async fn top_pools_24h(pool: &PgPool, venue: i16, limit: i64) -> eyre::Result<Vec<PoolRanking>> {
    let rows = sqlx::query_as!(
        PoolRanking,
        r#"
        SELECT
            p.pool_address, p.venue, p.token_x, p.token_y, p.tvl_usd, p.tier,
            i.bucket_start, i.quality, i.regime, i.r_org, i.top_score, i.vol_tvl, i.fee_tvl
        FROM indicators_24h i
        JOIN pools p ON p.pool_address = i.pool_address
        WHERE p.venue = $1
          AND i.bucket_start = (SELECT max(bucket_start) FROM indicators_24h)
        ORDER BY i.r_org DESC NULLS LAST
        LIMIT $2
        "#,
        venue,
        limit,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| "Querying top pools from indicators_24h")?;

    Ok(rows)
}

// Ranks the most recent bucket of the given timeframe by r_org -- the dimensionless fee-over-LVR
// ratio that is directly comparable across venues once a second venue exists.
pub async fn top_pools(
    pool: &PgPool,
    venue: i16,
    timeframe: Timeframe,
    limit: i64,
) -> eyre::Result<Vec<PoolRanking>> {
    match timeframe {
        Timeframe::M5 => top_pools_5m(pool, venue, limit).await,
        Timeframe::M10 => top_pools_10m(pool, venue, limit).await,
        Timeframe::H1 => top_pools_1h(pool, venue, limit).await,
        Timeframe::H4 => top_pools_4h(pool, venue, limit).await,
        Timeframe::H24 => top_pools_24h(pool, venue, limit).await,
    }
}
