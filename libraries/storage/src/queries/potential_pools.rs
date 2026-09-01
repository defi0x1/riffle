use eyre::WrapErr;
use sqlx::PgPool;

use crate::queries::PoolRanking;
use crate::types::{Timeframe, quality};

#[derive(Clone, Debug)]
pub struct PotentialPoolFilters {
    // Screening-quality rows are excluded by default: r_org from a TVL x phi_shape estimate is
    // a materially weaker claim than one measured from bin state, and potential_pools feeds
    // decisions that open a paper position.
    pub measured_only: bool,
    // Breakeven is r_org = 1 by construction; below it, fee income does not cover LVR.
    pub min_r_org: f64,
    pub regime: Option<String>,
}

impl Default for PotentialPoolFilters {
    fn default() -> Self {
        PotentialPoolFilters {
            measured_only: true,
            min_r_org: 1.0,
            regime: None,
        }
    }
}

async fn potential_pools_5m(
    pool: &PgPool,
    venue: i16,
    filters: &PotentialPoolFilters,
) -> eyre::Result<Vec<PoolRanking>> {
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
          AND (NOT $2 OR i.quality = $3)
          AND i.r_org >= $4
          AND ($5::text IS NULL OR i.regime = $5)
        ORDER BY i.r_org DESC NULLS LAST
        "#,
        venue,
        filters.measured_only,
        quality::MEASURED,
        filters.min_r_org,
        filters.regime,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| "Querying potential pools from indicators_5m")?;

    Ok(rows)
}

async fn potential_pools_10m(
    pool: &PgPool,
    venue: i16,
    filters: &PotentialPoolFilters,
) -> eyre::Result<Vec<PoolRanking>> {
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
          AND (NOT $2 OR i.quality = $3)
          AND i.r_org >= $4
          AND ($5::text IS NULL OR i.regime = $5)
        ORDER BY i.r_org DESC NULLS LAST
        "#,
        venue,
        filters.measured_only,
        quality::MEASURED,
        filters.min_r_org,
        filters.regime,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| "Querying potential pools from indicators_10m")?;

    Ok(rows)
}

async fn potential_pools_1h(
    pool: &PgPool,
    venue: i16,
    filters: &PotentialPoolFilters,
) -> eyre::Result<Vec<PoolRanking>> {
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
          AND (NOT $2 OR i.quality = $3)
          AND i.r_org >= $4
          AND ($5::text IS NULL OR i.regime = $5)
        ORDER BY i.r_org DESC NULLS LAST
        "#,
        venue,
        filters.measured_only,
        quality::MEASURED,
        filters.min_r_org,
        filters.regime,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| "Querying potential pools from indicators_1h")?;

    Ok(rows)
}

async fn potential_pools_4h(
    pool: &PgPool,
    venue: i16,
    filters: &PotentialPoolFilters,
) -> eyre::Result<Vec<PoolRanking>> {
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
          AND (NOT $2 OR i.quality = $3)
          AND i.r_org >= $4
          AND ($5::text IS NULL OR i.regime = $5)
        ORDER BY i.r_org DESC NULLS LAST
        "#,
        venue,
        filters.measured_only,
        quality::MEASURED,
        filters.min_r_org,
        filters.regime,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| "Querying potential pools from indicators_4h")?;

    Ok(rows)
}

async fn potential_pools_24h(
    pool: &PgPool,
    venue: i16,
    filters: &PotentialPoolFilters,
) -> eyre::Result<Vec<PoolRanking>> {
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
          AND (NOT $2 OR i.quality = $3)
          AND i.r_org >= $4
          AND ($5::text IS NULL OR i.regime = $5)
        ORDER BY i.r_org DESC NULLS LAST
        "#,
        venue,
        filters.measured_only,
        quality::MEASURED,
        filters.min_r_org,
        filters.regime,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| "Querying potential pools from indicators_24h")?;

    Ok(rows)
}

// Pools worth evaluating for a paper position at this timeframe: ranked, gate-passing by the
// caller's own filters. This is a narrower cut of top_pools, not a different ranking.
pub async fn potential_pools(
    pool: &PgPool,
    venue: i16,
    timeframe: Timeframe,
    filters: &PotentialPoolFilters,
) -> eyre::Result<Vec<PoolRanking>> {
    match timeframe {
        Timeframe::M5 => potential_pools_5m(pool, venue, filters).await,
        Timeframe::M10 => potential_pools_10m(pool, venue, filters).await,
        Timeframe::H1 => potential_pools_1h(pool, venue, filters).await,
        Timeframe::H4 => potential_pools_4h(pool, venue, filters).await,
        Timeframe::H24 => potential_pools_24h(pool, venue, filters).await,
    }
}
