use chrono::{DateTime, Utc};
use eyre::WrapErr;
use rust_decimal::Decimal;
use sqlx::PgPool;

// Everything the pipeline needs to screen or rank a pool, in one row: pool + DLMM params +
// both mints' risk-relevant fields. Two LEFT JOINs rather than a second round trip per pool --
// this query already runs over the whole universe.
#[derive(Clone, Debug)]
pub struct PoolForScoring {
    pub pool_address: String,
    pub venue: i16,
    pub tier: i16,
    pub token_x: String,
    pub token_y: String,
    pub bin_step: i16,
    pub base_factor: i32,
    pub variable_fee_control: i32,
    pub protocol_share_bps: i32,
    pub base_fee_bps: Decimal,
    pub tvl_usd: Option<Decimal>,
    pub status: i16,
    pub activation_point: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub first_liquidity_at: Option<DateTime<Utc>>,
    pub is_blacklisted: bool,

    pub x_mint_authority: Option<String>,
    pub x_freeze_authority: Option<String>,
    pub x_top10_share: Option<f64>,
    pub x_top1_share: Option<f64>,

    pub y_mint_authority: Option<String>,
    pub y_freeze_authority: Option<String>,
    pub y_top10_share: Option<f64>,
    pub y_top1_share: Option<f64>,
}

pub async fn scoring_universe(pool: &PgPool, venue: i16) -> eyre::Result<Vec<PoolForScoring>> {
    let rows = sqlx::query_as!(
        PoolForScoring,
        r#"
        SELECT
            p.pool_address, p.venue, p.tier, p.token_x, p.token_y,
            d.bin_step, d.base_factor, d.variable_fee_control,
            p.protocol_share_bps, p.base_fee_bps, p.tvl_usd, p.status,
            p.activation_point, p.created_at, p.first_liquidity_at, p.is_blacklisted,
            tx.mint_authority AS x_mint_authority, tx.freeze_authority AS x_freeze_authority,
            tx.top10_share AS x_top10_share, tx.top1_share AS x_top1_share,
            ty.mint_authority AS y_mint_authority, ty.freeze_authority AS y_freeze_authority,
            ty.top10_share AS y_top10_share, ty.top1_share AS y_top1_share
        FROM pools p
        JOIN dlmm_pool_params d ON d.pool_address = p.pool_address
        LEFT JOIN tokens tx ON tx.mint = p.token_x
        LEFT JOIN tokens ty ON ty.mint = p.token_y
        WHERE p.venue = $1
        ORDER BY p.pool_address
        "#,
        venue,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| "Querying scoring universe")?;

    Ok(rows)
}
