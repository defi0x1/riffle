use chrono::{DateTime, Utc};
use eyre::WrapErr;
use rust_decimal::Decimal;
use sqlx::PgPool;

use crate::types::Timeframe;

#[derive(Clone, Debug)]
pub struct IndicatorRow {
    pub pool_address: String,
    pub venue: i16,
    pub bucket_start: DateTime<Utc>,
    // "A" (measured, tier 1) or "B" (screening estimate, tier 0) -- see types::quality.
    pub quality: String,
    pub regime: Option<String>,
    pub vol_change: Option<f64>,
    pub fee_change: Option<f64>,
    pub tvl_change: Option<f64>,
    pub price_change: Option<f64>,
    pub active_tvl_change: Option<f64>,
    pub holders_change: Option<f64>,
    pub vol_tvl: Option<f64>,
    pub fee_tvl: Option<f64>,
    pub fee_active_tvl: Option<f64>,
    pub tau_a: Option<f64>,
    pub sigma_gk: Option<f64>,
    pub sigma_fast: Option<f64>,
    pub sigma_slow: Option<f64>,
    pub sigma_d: Option<f64>,
    pub sigma_jump: Option<f64>,
    pub f_hat: Option<Decimal>,
    pub phi_org: Option<f64>,
    pub phi_mech: Option<f64>,
    pub phi_time: Option<f64>,
    pub phi_size: Option<f64>,
    pub r_gross: Option<f64>,
    pub r_org: Option<f64>,
    pub y_fee: Option<f64>,
    // Reproduction of the venue's own weighted-percentile ranking, kept for comparison against
    // r_org rather than used for our own ranking decisions.
    pub top_score: Option<f64>,
}

pub async fn upsert_indicators_5m(pool: &PgPool, rows: &[IndicatorRow]) -> eyre::Result<u64> {
    if rows.is_empty() {
        return Ok(0);
    }

    let pool_address: Vec<&str> = rows.iter().map(|r| r.pool_address.as_str()).collect();
    let venue: Vec<i16> = rows.iter().map(|r| r.venue).collect();
    let bucket_start: Vec<DateTime<Utc>> = rows.iter().map(|r| r.bucket_start).collect();
    let quality: Vec<&str> = rows.iter().map(|r| r.quality.as_str()).collect();
    let regime: Vec<Option<&str>> = rows.iter().map(|r| r.regime.as_deref()).collect();
    let vol_change: Vec<Option<f64>> = rows.iter().map(|r| r.vol_change).collect();
    let fee_change: Vec<Option<f64>> = rows.iter().map(|r| r.fee_change).collect();
    let tvl_change: Vec<Option<f64>> = rows.iter().map(|r| r.tvl_change).collect();
    let price_change: Vec<Option<f64>> = rows.iter().map(|r| r.price_change).collect();
    let active_tvl_change: Vec<Option<f64>> = rows.iter().map(|r| r.active_tvl_change).collect();
    let holders_change: Vec<Option<f64>> = rows.iter().map(|r| r.holders_change).collect();
    let vol_tvl: Vec<Option<f64>> = rows.iter().map(|r| r.vol_tvl).collect();
    let fee_tvl: Vec<Option<f64>> = rows.iter().map(|r| r.fee_tvl).collect();
    let fee_active_tvl: Vec<Option<f64>> = rows.iter().map(|r| r.fee_active_tvl).collect();
    let tau_a: Vec<Option<f64>> = rows.iter().map(|r| r.tau_a).collect();
    let sigma_gk: Vec<Option<f64>> = rows.iter().map(|r| r.sigma_gk).collect();
    let sigma_fast: Vec<Option<f64>> = rows.iter().map(|r| r.sigma_fast).collect();
    let sigma_slow: Vec<Option<f64>> = rows.iter().map(|r| r.sigma_slow).collect();
    let sigma_d: Vec<Option<f64>> = rows.iter().map(|r| r.sigma_d).collect();
    let sigma_jump: Vec<Option<f64>> = rows.iter().map(|r| r.sigma_jump).collect();
    let f_hat: Vec<Option<Decimal>> = rows.iter().map(|r| r.f_hat).collect();
    let phi_org: Vec<Option<f64>> = rows.iter().map(|r| r.phi_org).collect();
    let phi_mech: Vec<Option<f64>> = rows.iter().map(|r| r.phi_mech).collect();
    let phi_time: Vec<Option<f64>> = rows.iter().map(|r| r.phi_time).collect();
    let phi_size: Vec<Option<f64>> = rows.iter().map(|r| r.phi_size).collect();
    let r_gross: Vec<Option<f64>> = rows.iter().map(|r| r.r_gross).collect();
    let r_org: Vec<Option<f64>> = rows.iter().map(|r| r.r_org).collect();
    let y_fee: Vec<Option<f64>> = rows.iter().map(|r| r.y_fee).collect();
    let top_score: Vec<Option<f64>> = rows.iter().map(|r| r.top_score).collect();

    let result = sqlx::query!(
        r#"
        INSERT INTO indicators_5m (
            pool_address, venue, bucket_start, quality, regime,
            vol_change, fee_change, tvl_change, price_change, active_tvl_change, holders_change,
            vol_tvl, fee_tvl, fee_active_tvl, tau_a,
            sigma_gk, sigma_fast, sigma_slow, sigma_d, sigma_jump,
            f_hat, phi_org, phi_mech, phi_time, phi_size, r_gross, r_org, y_fee, top_score
        )
        SELECT * FROM UNNEST(
            $1::text[], $2::smallint[], $3::timestamptz[], $4::text[], $5::text[],
            $6::float8[], $7::float8[], $8::float8[], $9::float8[], $10::float8[], $11::float8[],
            $12::float8[], $13::float8[], $14::float8[], $15::float8[],
            $16::float8[], $17::float8[], $18::float8[], $19::float8[], $20::float8[],
            $21::numeric[], $22::float8[], $23::float8[], $24::float8[], $25::float8[],
            $26::float8[], $27::float8[], $28::float8[], $29::float8[]
        )
        ON CONFLICT (pool_address, bucket_start) DO UPDATE SET
            venue              = EXCLUDED.venue,
            quality            = EXCLUDED.quality,
            regime             = EXCLUDED.regime,
            vol_change         = EXCLUDED.vol_change,
            fee_change         = EXCLUDED.fee_change,
            tvl_change         = EXCLUDED.tvl_change,
            price_change       = EXCLUDED.price_change,
            active_tvl_change  = EXCLUDED.active_tvl_change,
            holders_change     = EXCLUDED.holders_change,
            vol_tvl            = EXCLUDED.vol_tvl,
            fee_tvl            = EXCLUDED.fee_tvl,
            fee_active_tvl     = EXCLUDED.fee_active_tvl,
            tau_a              = EXCLUDED.tau_a,
            sigma_gk           = EXCLUDED.sigma_gk,
            sigma_fast         = EXCLUDED.sigma_fast,
            sigma_slow         = EXCLUDED.sigma_slow,
            sigma_d            = EXCLUDED.sigma_d,
            sigma_jump         = EXCLUDED.sigma_jump,
            f_hat              = EXCLUDED.f_hat,
            phi_org            = EXCLUDED.phi_org,
            phi_mech           = EXCLUDED.phi_mech,
            phi_time           = EXCLUDED.phi_time,
            phi_size           = EXCLUDED.phi_size,
            r_gross            = EXCLUDED.r_gross,
            r_org              = EXCLUDED.r_org,
            y_fee              = EXCLUDED.y_fee,
            top_score          = EXCLUDED.top_score
        "#,
        &pool_address as &[&str],
        &venue,
        &bucket_start,
        &quality as &[&str],
        &regime as &[Option<&str>],
        &vol_change as &[Option<f64>],
        &fee_change as &[Option<f64>],
        &tvl_change as &[Option<f64>],
        &price_change as &[Option<f64>],
        &active_tvl_change as &[Option<f64>],
        &holders_change as &[Option<f64>],
        &vol_tvl as &[Option<f64>],
        &fee_tvl as &[Option<f64>],
        &fee_active_tvl as &[Option<f64>],
        &tau_a as &[Option<f64>],
        &sigma_gk as &[Option<f64>],
        &sigma_fast as &[Option<f64>],
        &sigma_slow as &[Option<f64>],
        &sigma_d as &[Option<f64>],
        &sigma_jump as &[Option<f64>],
        &f_hat as &[Option<Decimal>],
        &phi_org as &[Option<f64>],
        &phi_mech as &[Option<f64>],
        &phi_time as &[Option<f64>],
        &phi_size as &[Option<f64>],
        &r_gross as &[Option<f64>],
        &r_org as &[Option<f64>],
        &y_fee as &[Option<f64>],
        &top_score as &[Option<f64>],
    )
    .execute(pool)
    .await
    .wrap_err_with(|| format!("Upserting {} rows into indicators_5m", rows.len()))?;

    Ok(result.rows_affected())
}

pub async fn upsert_indicators_10m(pool: &PgPool, rows: &[IndicatorRow]) -> eyre::Result<u64> {
    if rows.is_empty() {
        return Ok(0);
    }

    let pool_address: Vec<&str> = rows.iter().map(|r| r.pool_address.as_str()).collect();
    let venue: Vec<i16> = rows.iter().map(|r| r.venue).collect();
    let bucket_start: Vec<DateTime<Utc>> = rows.iter().map(|r| r.bucket_start).collect();
    let quality: Vec<&str> = rows.iter().map(|r| r.quality.as_str()).collect();
    let regime: Vec<Option<&str>> = rows.iter().map(|r| r.regime.as_deref()).collect();
    let vol_change: Vec<Option<f64>> = rows.iter().map(|r| r.vol_change).collect();
    let fee_change: Vec<Option<f64>> = rows.iter().map(|r| r.fee_change).collect();
    let tvl_change: Vec<Option<f64>> = rows.iter().map(|r| r.tvl_change).collect();
    let price_change: Vec<Option<f64>> = rows.iter().map(|r| r.price_change).collect();
    let active_tvl_change: Vec<Option<f64>> = rows.iter().map(|r| r.active_tvl_change).collect();
    let holders_change: Vec<Option<f64>> = rows.iter().map(|r| r.holders_change).collect();
    let vol_tvl: Vec<Option<f64>> = rows.iter().map(|r| r.vol_tvl).collect();
    let fee_tvl: Vec<Option<f64>> = rows.iter().map(|r| r.fee_tvl).collect();
    let fee_active_tvl: Vec<Option<f64>> = rows.iter().map(|r| r.fee_active_tvl).collect();
    let tau_a: Vec<Option<f64>> = rows.iter().map(|r| r.tau_a).collect();
    let sigma_gk: Vec<Option<f64>> = rows.iter().map(|r| r.sigma_gk).collect();
    let sigma_fast: Vec<Option<f64>> = rows.iter().map(|r| r.sigma_fast).collect();
    let sigma_slow: Vec<Option<f64>> = rows.iter().map(|r| r.sigma_slow).collect();
    let sigma_d: Vec<Option<f64>> = rows.iter().map(|r| r.sigma_d).collect();
    let sigma_jump: Vec<Option<f64>> = rows.iter().map(|r| r.sigma_jump).collect();
    let f_hat: Vec<Option<Decimal>> = rows.iter().map(|r| r.f_hat).collect();
    let phi_org: Vec<Option<f64>> = rows.iter().map(|r| r.phi_org).collect();
    let phi_mech: Vec<Option<f64>> = rows.iter().map(|r| r.phi_mech).collect();
    let phi_time: Vec<Option<f64>> = rows.iter().map(|r| r.phi_time).collect();
    let phi_size: Vec<Option<f64>> = rows.iter().map(|r| r.phi_size).collect();
    let r_gross: Vec<Option<f64>> = rows.iter().map(|r| r.r_gross).collect();
    let r_org: Vec<Option<f64>> = rows.iter().map(|r| r.r_org).collect();
    let y_fee: Vec<Option<f64>> = rows.iter().map(|r| r.y_fee).collect();
    let top_score: Vec<Option<f64>> = rows.iter().map(|r| r.top_score).collect();

    let result = sqlx::query!(
        r#"
        INSERT INTO indicators_10m (
            pool_address, venue, bucket_start, quality, regime,
            vol_change, fee_change, tvl_change, price_change, active_tvl_change, holders_change,
            vol_tvl, fee_tvl, fee_active_tvl, tau_a,
            sigma_gk, sigma_fast, sigma_slow, sigma_d, sigma_jump,
            f_hat, phi_org, phi_mech, phi_time, phi_size, r_gross, r_org, y_fee, top_score
        )
        SELECT * FROM UNNEST(
            $1::text[], $2::smallint[], $3::timestamptz[], $4::text[], $5::text[],
            $6::float8[], $7::float8[], $8::float8[], $9::float8[], $10::float8[], $11::float8[],
            $12::float8[], $13::float8[], $14::float8[], $15::float8[],
            $16::float8[], $17::float8[], $18::float8[], $19::float8[], $20::float8[],
            $21::numeric[], $22::float8[], $23::float8[], $24::float8[], $25::float8[],
            $26::float8[], $27::float8[], $28::float8[], $29::float8[]
        )
        ON CONFLICT (pool_address, bucket_start) DO UPDATE SET
            venue              = EXCLUDED.venue,
            quality            = EXCLUDED.quality,
            regime             = EXCLUDED.regime,
            vol_change         = EXCLUDED.vol_change,
            fee_change         = EXCLUDED.fee_change,
            tvl_change         = EXCLUDED.tvl_change,
            price_change       = EXCLUDED.price_change,
            active_tvl_change  = EXCLUDED.active_tvl_change,
            holders_change     = EXCLUDED.holders_change,
            vol_tvl            = EXCLUDED.vol_tvl,
            fee_tvl            = EXCLUDED.fee_tvl,
            fee_active_tvl     = EXCLUDED.fee_active_tvl,
            tau_a              = EXCLUDED.tau_a,
            sigma_gk           = EXCLUDED.sigma_gk,
            sigma_fast         = EXCLUDED.sigma_fast,
            sigma_slow         = EXCLUDED.sigma_slow,
            sigma_d            = EXCLUDED.sigma_d,
            sigma_jump         = EXCLUDED.sigma_jump,
            f_hat              = EXCLUDED.f_hat,
            phi_org            = EXCLUDED.phi_org,
            phi_mech           = EXCLUDED.phi_mech,
            phi_time           = EXCLUDED.phi_time,
            phi_size           = EXCLUDED.phi_size,
            r_gross            = EXCLUDED.r_gross,
            r_org              = EXCLUDED.r_org,
            y_fee              = EXCLUDED.y_fee,
            top_score          = EXCLUDED.top_score
        "#,
        &pool_address as &[&str],
        &venue,
        &bucket_start,
        &quality as &[&str],
        &regime as &[Option<&str>],
        &vol_change as &[Option<f64>],
        &fee_change as &[Option<f64>],
        &tvl_change as &[Option<f64>],
        &price_change as &[Option<f64>],
        &active_tvl_change as &[Option<f64>],
        &holders_change as &[Option<f64>],
        &vol_tvl as &[Option<f64>],
        &fee_tvl as &[Option<f64>],
        &fee_active_tvl as &[Option<f64>],
        &tau_a as &[Option<f64>],
        &sigma_gk as &[Option<f64>],
        &sigma_fast as &[Option<f64>],
        &sigma_slow as &[Option<f64>],
        &sigma_d as &[Option<f64>],
        &sigma_jump as &[Option<f64>],
        &f_hat as &[Option<Decimal>],
        &phi_org as &[Option<f64>],
        &phi_mech as &[Option<f64>],
        &phi_time as &[Option<f64>],
        &phi_size as &[Option<f64>],
        &r_gross as &[Option<f64>],
        &r_org as &[Option<f64>],
        &y_fee as &[Option<f64>],
        &top_score as &[Option<f64>],
    )
    .execute(pool)
    .await
    .wrap_err_with(|| format!("Upserting {} rows into indicators_10m", rows.len()))?;

    Ok(result.rows_affected())
}

pub async fn upsert_indicators_1h(pool: &PgPool, rows: &[IndicatorRow]) -> eyre::Result<u64> {
    if rows.is_empty() {
        return Ok(0);
    }

    let pool_address: Vec<&str> = rows.iter().map(|r| r.pool_address.as_str()).collect();
    let venue: Vec<i16> = rows.iter().map(|r| r.venue).collect();
    let bucket_start: Vec<DateTime<Utc>> = rows.iter().map(|r| r.bucket_start).collect();
    let quality: Vec<&str> = rows.iter().map(|r| r.quality.as_str()).collect();
    let regime: Vec<Option<&str>> = rows.iter().map(|r| r.regime.as_deref()).collect();
    let vol_change: Vec<Option<f64>> = rows.iter().map(|r| r.vol_change).collect();
    let fee_change: Vec<Option<f64>> = rows.iter().map(|r| r.fee_change).collect();
    let tvl_change: Vec<Option<f64>> = rows.iter().map(|r| r.tvl_change).collect();
    let price_change: Vec<Option<f64>> = rows.iter().map(|r| r.price_change).collect();
    let active_tvl_change: Vec<Option<f64>> = rows.iter().map(|r| r.active_tvl_change).collect();
    let holders_change: Vec<Option<f64>> = rows.iter().map(|r| r.holders_change).collect();
    let vol_tvl: Vec<Option<f64>> = rows.iter().map(|r| r.vol_tvl).collect();
    let fee_tvl: Vec<Option<f64>> = rows.iter().map(|r| r.fee_tvl).collect();
    let fee_active_tvl: Vec<Option<f64>> = rows.iter().map(|r| r.fee_active_tvl).collect();
    let tau_a: Vec<Option<f64>> = rows.iter().map(|r| r.tau_a).collect();
    let sigma_gk: Vec<Option<f64>> = rows.iter().map(|r| r.sigma_gk).collect();
    let sigma_fast: Vec<Option<f64>> = rows.iter().map(|r| r.sigma_fast).collect();
    let sigma_slow: Vec<Option<f64>> = rows.iter().map(|r| r.sigma_slow).collect();
    let sigma_d: Vec<Option<f64>> = rows.iter().map(|r| r.sigma_d).collect();
    let sigma_jump: Vec<Option<f64>> = rows.iter().map(|r| r.sigma_jump).collect();
    let f_hat: Vec<Option<Decimal>> = rows.iter().map(|r| r.f_hat).collect();
    let phi_org: Vec<Option<f64>> = rows.iter().map(|r| r.phi_org).collect();
    let phi_mech: Vec<Option<f64>> = rows.iter().map(|r| r.phi_mech).collect();
    let phi_time: Vec<Option<f64>> = rows.iter().map(|r| r.phi_time).collect();
    let phi_size: Vec<Option<f64>> = rows.iter().map(|r| r.phi_size).collect();
    let r_gross: Vec<Option<f64>> = rows.iter().map(|r| r.r_gross).collect();
    let r_org: Vec<Option<f64>> = rows.iter().map(|r| r.r_org).collect();
    let y_fee: Vec<Option<f64>> = rows.iter().map(|r| r.y_fee).collect();
    let top_score: Vec<Option<f64>> = rows.iter().map(|r| r.top_score).collect();

    let result = sqlx::query!(
        r#"
        INSERT INTO indicators_1h (
            pool_address, venue, bucket_start, quality, regime,
            vol_change, fee_change, tvl_change, price_change, active_tvl_change, holders_change,
            vol_tvl, fee_tvl, fee_active_tvl, tau_a,
            sigma_gk, sigma_fast, sigma_slow, sigma_d, sigma_jump,
            f_hat, phi_org, phi_mech, phi_time, phi_size, r_gross, r_org, y_fee, top_score
        )
        SELECT * FROM UNNEST(
            $1::text[], $2::smallint[], $3::timestamptz[], $4::text[], $5::text[],
            $6::float8[], $7::float8[], $8::float8[], $9::float8[], $10::float8[], $11::float8[],
            $12::float8[], $13::float8[], $14::float8[], $15::float8[],
            $16::float8[], $17::float8[], $18::float8[], $19::float8[], $20::float8[],
            $21::numeric[], $22::float8[], $23::float8[], $24::float8[], $25::float8[],
            $26::float8[], $27::float8[], $28::float8[], $29::float8[]
        )
        ON CONFLICT (pool_address, bucket_start) DO UPDATE SET
            venue              = EXCLUDED.venue,
            quality            = EXCLUDED.quality,
            regime             = EXCLUDED.regime,
            vol_change         = EXCLUDED.vol_change,
            fee_change         = EXCLUDED.fee_change,
            tvl_change         = EXCLUDED.tvl_change,
            price_change       = EXCLUDED.price_change,
            active_tvl_change  = EXCLUDED.active_tvl_change,
            holders_change     = EXCLUDED.holders_change,
            vol_tvl            = EXCLUDED.vol_tvl,
            fee_tvl            = EXCLUDED.fee_tvl,
            fee_active_tvl     = EXCLUDED.fee_active_tvl,
            tau_a              = EXCLUDED.tau_a,
            sigma_gk           = EXCLUDED.sigma_gk,
            sigma_fast         = EXCLUDED.sigma_fast,
            sigma_slow         = EXCLUDED.sigma_slow,
            sigma_d            = EXCLUDED.sigma_d,
            sigma_jump         = EXCLUDED.sigma_jump,
            f_hat              = EXCLUDED.f_hat,
            phi_org            = EXCLUDED.phi_org,
            phi_mech           = EXCLUDED.phi_mech,
            phi_time           = EXCLUDED.phi_time,
            phi_size           = EXCLUDED.phi_size,
            r_gross            = EXCLUDED.r_gross,
            r_org              = EXCLUDED.r_org,
            y_fee              = EXCLUDED.y_fee,
            top_score          = EXCLUDED.top_score
        "#,
        &pool_address as &[&str],
        &venue,
        &bucket_start,
        &quality as &[&str],
        &regime as &[Option<&str>],
        &vol_change as &[Option<f64>],
        &fee_change as &[Option<f64>],
        &tvl_change as &[Option<f64>],
        &price_change as &[Option<f64>],
        &active_tvl_change as &[Option<f64>],
        &holders_change as &[Option<f64>],
        &vol_tvl as &[Option<f64>],
        &fee_tvl as &[Option<f64>],
        &fee_active_tvl as &[Option<f64>],
        &tau_a as &[Option<f64>],
        &sigma_gk as &[Option<f64>],
        &sigma_fast as &[Option<f64>],
        &sigma_slow as &[Option<f64>],
        &sigma_d as &[Option<f64>],
        &sigma_jump as &[Option<f64>],
        &f_hat as &[Option<Decimal>],
        &phi_org as &[Option<f64>],
        &phi_mech as &[Option<f64>],
        &phi_time as &[Option<f64>],
        &phi_size as &[Option<f64>],
        &r_gross as &[Option<f64>],
        &r_org as &[Option<f64>],
        &y_fee as &[Option<f64>],
        &top_score as &[Option<f64>],
    )
    .execute(pool)
    .await
    .wrap_err_with(|| format!("Upserting {} rows into indicators_1h", rows.len()))?;

    Ok(result.rows_affected())
}

pub async fn upsert_indicators_4h(pool: &PgPool, rows: &[IndicatorRow]) -> eyre::Result<u64> {
    if rows.is_empty() {
        return Ok(0);
    }

    let pool_address: Vec<&str> = rows.iter().map(|r| r.pool_address.as_str()).collect();
    let venue: Vec<i16> = rows.iter().map(|r| r.venue).collect();
    let bucket_start: Vec<DateTime<Utc>> = rows.iter().map(|r| r.bucket_start).collect();
    let quality: Vec<&str> = rows.iter().map(|r| r.quality.as_str()).collect();
    let regime: Vec<Option<&str>> = rows.iter().map(|r| r.regime.as_deref()).collect();
    let vol_change: Vec<Option<f64>> = rows.iter().map(|r| r.vol_change).collect();
    let fee_change: Vec<Option<f64>> = rows.iter().map(|r| r.fee_change).collect();
    let tvl_change: Vec<Option<f64>> = rows.iter().map(|r| r.tvl_change).collect();
    let price_change: Vec<Option<f64>> = rows.iter().map(|r| r.price_change).collect();
    let active_tvl_change: Vec<Option<f64>> = rows.iter().map(|r| r.active_tvl_change).collect();
    let holders_change: Vec<Option<f64>> = rows.iter().map(|r| r.holders_change).collect();
    let vol_tvl: Vec<Option<f64>> = rows.iter().map(|r| r.vol_tvl).collect();
    let fee_tvl: Vec<Option<f64>> = rows.iter().map(|r| r.fee_tvl).collect();
    let fee_active_tvl: Vec<Option<f64>> = rows.iter().map(|r| r.fee_active_tvl).collect();
    let tau_a: Vec<Option<f64>> = rows.iter().map(|r| r.tau_a).collect();
    let sigma_gk: Vec<Option<f64>> = rows.iter().map(|r| r.sigma_gk).collect();
    let sigma_fast: Vec<Option<f64>> = rows.iter().map(|r| r.sigma_fast).collect();
    let sigma_slow: Vec<Option<f64>> = rows.iter().map(|r| r.sigma_slow).collect();
    let sigma_d: Vec<Option<f64>> = rows.iter().map(|r| r.sigma_d).collect();
    let sigma_jump: Vec<Option<f64>> = rows.iter().map(|r| r.sigma_jump).collect();
    let f_hat: Vec<Option<Decimal>> = rows.iter().map(|r| r.f_hat).collect();
    let phi_org: Vec<Option<f64>> = rows.iter().map(|r| r.phi_org).collect();
    let phi_mech: Vec<Option<f64>> = rows.iter().map(|r| r.phi_mech).collect();
    let phi_time: Vec<Option<f64>> = rows.iter().map(|r| r.phi_time).collect();
    let phi_size: Vec<Option<f64>> = rows.iter().map(|r| r.phi_size).collect();
    let r_gross: Vec<Option<f64>> = rows.iter().map(|r| r.r_gross).collect();
    let r_org: Vec<Option<f64>> = rows.iter().map(|r| r.r_org).collect();
    let y_fee: Vec<Option<f64>> = rows.iter().map(|r| r.y_fee).collect();
    let top_score: Vec<Option<f64>> = rows.iter().map(|r| r.top_score).collect();

    let result = sqlx::query!(
        r#"
        INSERT INTO indicators_4h (
            pool_address, venue, bucket_start, quality, regime,
            vol_change, fee_change, tvl_change, price_change, active_tvl_change, holders_change,
            vol_tvl, fee_tvl, fee_active_tvl, tau_a,
            sigma_gk, sigma_fast, sigma_slow, sigma_d, sigma_jump,
            f_hat, phi_org, phi_mech, phi_time, phi_size, r_gross, r_org, y_fee, top_score
        )
        SELECT * FROM UNNEST(
            $1::text[], $2::smallint[], $3::timestamptz[], $4::text[], $5::text[],
            $6::float8[], $7::float8[], $8::float8[], $9::float8[], $10::float8[], $11::float8[],
            $12::float8[], $13::float8[], $14::float8[], $15::float8[],
            $16::float8[], $17::float8[], $18::float8[], $19::float8[], $20::float8[],
            $21::numeric[], $22::float8[], $23::float8[], $24::float8[], $25::float8[],
            $26::float8[], $27::float8[], $28::float8[], $29::float8[]
        )
        ON CONFLICT (pool_address, bucket_start) DO UPDATE SET
            venue              = EXCLUDED.venue,
            quality            = EXCLUDED.quality,
            regime             = EXCLUDED.regime,
            vol_change         = EXCLUDED.vol_change,
            fee_change         = EXCLUDED.fee_change,
            tvl_change         = EXCLUDED.tvl_change,
            price_change       = EXCLUDED.price_change,
            active_tvl_change  = EXCLUDED.active_tvl_change,
            holders_change     = EXCLUDED.holders_change,
            vol_tvl            = EXCLUDED.vol_tvl,
            fee_tvl            = EXCLUDED.fee_tvl,
            fee_active_tvl     = EXCLUDED.fee_active_tvl,
            tau_a              = EXCLUDED.tau_a,
            sigma_gk           = EXCLUDED.sigma_gk,
            sigma_fast         = EXCLUDED.sigma_fast,
            sigma_slow         = EXCLUDED.sigma_slow,
            sigma_d            = EXCLUDED.sigma_d,
            sigma_jump         = EXCLUDED.sigma_jump,
            f_hat              = EXCLUDED.f_hat,
            phi_org            = EXCLUDED.phi_org,
            phi_mech           = EXCLUDED.phi_mech,
            phi_time           = EXCLUDED.phi_time,
            phi_size           = EXCLUDED.phi_size,
            r_gross            = EXCLUDED.r_gross,
            r_org              = EXCLUDED.r_org,
            y_fee              = EXCLUDED.y_fee,
            top_score          = EXCLUDED.top_score
        "#,
        &pool_address as &[&str],
        &venue,
        &bucket_start,
        &quality as &[&str],
        &regime as &[Option<&str>],
        &vol_change as &[Option<f64>],
        &fee_change as &[Option<f64>],
        &tvl_change as &[Option<f64>],
        &price_change as &[Option<f64>],
        &active_tvl_change as &[Option<f64>],
        &holders_change as &[Option<f64>],
        &vol_tvl as &[Option<f64>],
        &fee_tvl as &[Option<f64>],
        &fee_active_tvl as &[Option<f64>],
        &tau_a as &[Option<f64>],
        &sigma_gk as &[Option<f64>],
        &sigma_fast as &[Option<f64>],
        &sigma_slow as &[Option<f64>],
        &sigma_d as &[Option<f64>],
        &sigma_jump as &[Option<f64>],
        &f_hat as &[Option<Decimal>],
        &phi_org as &[Option<f64>],
        &phi_mech as &[Option<f64>],
        &phi_time as &[Option<f64>],
        &phi_size as &[Option<f64>],
        &r_gross as &[Option<f64>],
        &r_org as &[Option<f64>],
        &y_fee as &[Option<f64>],
        &top_score as &[Option<f64>],
    )
    .execute(pool)
    .await
    .wrap_err_with(|| format!("Upserting {} rows into indicators_4h", rows.len()))?;

    Ok(result.rows_affected())
}

pub async fn upsert_indicators_24h(pool: &PgPool, rows: &[IndicatorRow]) -> eyre::Result<u64> {
    if rows.is_empty() {
        return Ok(0);
    }

    let pool_address: Vec<&str> = rows.iter().map(|r| r.pool_address.as_str()).collect();
    let venue: Vec<i16> = rows.iter().map(|r| r.venue).collect();
    let bucket_start: Vec<DateTime<Utc>> = rows.iter().map(|r| r.bucket_start).collect();
    let quality: Vec<&str> = rows.iter().map(|r| r.quality.as_str()).collect();
    let regime: Vec<Option<&str>> = rows.iter().map(|r| r.regime.as_deref()).collect();
    let vol_change: Vec<Option<f64>> = rows.iter().map(|r| r.vol_change).collect();
    let fee_change: Vec<Option<f64>> = rows.iter().map(|r| r.fee_change).collect();
    let tvl_change: Vec<Option<f64>> = rows.iter().map(|r| r.tvl_change).collect();
    let price_change: Vec<Option<f64>> = rows.iter().map(|r| r.price_change).collect();
    let active_tvl_change: Vec<Option<f64>> = rows.iter().map(|r| r.active_tvl_change).collect();
    let holders_change: Vec<Option<f64>> = rows.iter().map(|r| r.holders_change).collect();
    let vol_tvl: Vec<Option<f64>> = rows.iter().map(|r| r.vol_tvl).collect();
    let fee_tvl: Vec<Option<f64>> = rows.iter().map(|r| r.fee_tvl).collect();
    let fee_active_tvl: Vec<Option<f64>> = rows.iter().map(|r| r.fee_active_tvl).collect();
    let tau_a: Vec<Option<f64>> = rows.iter().map(|r| r.tau_a).collect();
    let sigma_gk: Vec<Option<f64>> = rows.iter().map(|r| r.sigma_gk).collect();
    let sigma_fast: Vec<Option<f64>> = rows.iter().map(|r| r.sigma_fast).collect();
    let sigma_slow: Vec<Option<f64>> = rows.iter().map(|r| r.sigma_slow).collect();
    let sigma_d: Vec<Option<f64>> = rows.iter().map(|r| r.sigma_d).collect();
    let sigma_jump: Vec<Option<f64>> = rows.iter().map(|r| r.sigma_jump).collect();
    let f_hat: Vec<Option<Decimal>> = rows.iter().map(|r| r.f_hat).collect();
    let phi_org: Vec<Option<f64>> = rows.iter().map(|r| r.phi_org).collect();
    let phi_mech: Vec<Option<f64>> = rows.iter().map(|r| r.phi_mech).collect();
    let phi_time: Vec<Option<f64>> = rows.iter().map(|r| r.phi_time).collect();
    let phi_size: Vec<Option<f64>> = rows.iter().map(|r| r.phi_size).collect();
    let r_gross: Vec<Option<f64>> = rows.iter().map(|r| r.r_gross).collect();
    let r_org: Vec<Option<f64>> = rows.iter().map(|r| r.r_org).collect();
    let y_fee: Vec<Option<f64>> = rows.iter().map(|r| r.y_fee).collect();
    let top_score: Vec<Option<f64>> = rows.iter().map(|r| r.top_score).collect();

    let result = sqlx::query!(
        r#"
        INSERT INTO indicators_24h (
            pool_address, venue, bucket_start, quality, regime,
            vol_change, fee_change, tvl_change, price_change, active_tvl_change, holders_change,
            vol_tvl, fee_tvl, fee_active_tvl, tau_a,
            sigma_gk, sigma_fast, sigma_slow, sigma_d, sigma_jump,
            f_hat, phi_org, phi_mech, phi_time, phi_size, r_gross, r_org, y_fee, top_score
        )
        SELECT * FROM UNNEST(
            $1::text[], $2::smallint[], $3::timestamptz[], $4::text[], $5::text[],
            $6::float8[], $7::float8[], $8::float8[], $9::float8[], $10::float8[], $11::float8[],
            $12::float8[], $13::float8[], $14::float8[], $15::float8[],
            $16::float8[], $17::float8[], $18::float8[], $19::float8[], $20::float8[],
            $21::numeric[], $22::float8[], $23::float8[], $24::float8[], $25::float8[],
            $26::float8[], $27::float8[], $28::float8[], $29::float8[]
        )
        ON CONFLICT (pool_address, bucket_start) DO UPDATE SET
            venue              = EXCLUDED.venue,
            quality            = EXCLUDED.quality,
            regime             = EXCLUDED.regime,
            vol_change         = EXCLUDED.vol_change,
            fee_change         = EXCLUDED.fee_change,
            tvl_change         = EXCLUDED.tvl_change,
            price_change       = EXCLUDED.price_change,
            active_tvl_change  = EXCLUDED.active_tvl_change,
            holders_change     = EXCLUDED.holders_change,
            vol_tvl            = EXCLUDED.vol_tvl,
            fee_tvl            = EXCLUDED.fee_tvl,
            fee_active_tvl     = EXCLUDED.fee_active_tvl,
            tau_a              = EXCLUDED.tau_a,
            sigma_gk           = EXCLUDED.sigma_gk,
            sigma_fast         = EXCLUDED.sigma_fast,
            sigma_slow         = EXCLUDED.sigma_slow,
            sigma_d            = EXCLUDED.sigma_d,
            sigma_jump         = EXCLUDED.sigma_jump,
            f_hat              = EXCLUDED.f_hat,
            phi_org            = EXCLUDED.phi_org,
            phi_mech           = EXCLUDED.phi_mech,
            phi_time           = EXCLUDED.phi_time,
            phi_size           = EXCLUDED.phi_size,
            r_gross            = EXCLUDED.r_gross,
            r_org              = EXCLUDED.r_org,
            y_fee              = EXCLUDED.y_fee,
            top_score          = EXCLUDED.top_score
        "#,
        &pool_address as &[&str],
        &venue,
        &bucket_start,
        &quality as &[&str],
        &regime as &[Option<&str>],
        &vol_change as &[Option<f64>],
        &fee_change as &[Option<f64>],
        &tvl_change as &[Option<f64>],
        &price_change as &[Option<f64>],
        &active_tvl_change as &[Option<f64>],
        &holders_change as &[Option<f64>],
        &vol_tvl as &[Option<f64>],
        &fee_tvl as &[Option<f64>],
        &fee_active_tvl as &[Option<f64>],
        &tau_a as &[Option<f64>],
        &sigma_gk as &[Option<f64>],
        &sigma_fast as &[Option<f64>],
        &sigma_slow as &[Option<f64>],
        &sigma_d as &[Option<f64>],
        &sigma_jump as &[Option<f64>],
        &f_hat as &[Option<Decimal>],
        &phi_org as &[Option<f64>],
        &phi_mech as &[Option<f64>],
        &phi_time as &[Option<f64>],
        &phi_size as &[Option<f64>],
        &r_gross as &[Option<f64>],
        &r_org as &[Option<f64>],
        &y_fee as &[Option<f64>],
        &top_score as &[Option<f64>],
    )
    .execute(pool)
    .await
    .wrap_err_with(|| format!("Upserting {} rows into indicators_24h", rows.len()))?;

    Ok(result.rows_affected())
}

// Dispatches to the literal per-table upsert a timeframe needs -- sqlx requires a literal table
// name, so the table cannot be a bind parameter.
pub async fn upsert_indicators(
    pool: &PgPool,
    timeframe: Timeframe,
    rows: &[IndicatorRow],
) -> eyre::Result<u64> {
    match timeframe {
        Timeframe::M5 => upsert_indicators_5m(pool, rows).await,
        Timeframe::M10 => upsert_indicators_10m(pool, rows).await,
        Timeframe::H1 => upsert_indicators_1h(pool, rows).await,
        Timeframe::H4 => upsert_indicators_4h(pool, rows).await,
        Timeframe::H24 => upsert_indicators_24h(pool, rows).await,
    }
}
