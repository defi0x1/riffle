use chrono::{DateTime, Utc};
use eyre::WrapErr;
use sqlx::PgPool;

#[derive(Clone, Debug)]
pub struct RegimeStateRow {
    pub regime: Option<String>,
    pub since: DateTime<Utc>,
    pub pending: Option<String>,
    pub pending_since: Option<DateTime<Utc>>,
    pub last_transition: Option<DateTime<Utc>>,
}

// Loaded once at worker startup (and on demand for a newly promoted pool) so the hysteresis
// clock survives a restart -- see `engine::regime::RegimeState`, which this mirrors field for
// field.
pub async fn load_regime_state(
    pool: &PgPool,
    pool_address: &str,
    venue: i16,
    timeframe: &str,
) -> eyre::Result<Option<RegimeStateRow>> {
    let row = sqlx::query_as!(
        RegimeStateRow,
        r#"
        SELECT regime, since, pending, pending_since, last_transition
        FROM regime_state
        WHERE pool_address = $1 AND venue = $2 AND timeframe = $3
        "#,
        pool_address,
        venue,
        timeframe,
    )
    .fetch_optional(pool)
    .await
    .wrap_err_with(|| format!("Loading regime state for {pool_address}/{timeframe}"))?;

    Ok(row)
}

#[derive(Clone, Debug)]
pub struct VolatilityStateRow {
    pub sigma_fast_variance: f64,
    pub sigma_slow_variance: f64,
    pub first_observed_at: DateTime<Utc>,
}

pub async fn load_volatility_state(
    pool: &PgPool,
    pool_address: &str,
    venue: i16,
    timeframe: &str,
) -> eyre::Result<Option<VolatilityStateRow>> {
    let row = sqlx::query_as!(
        VolatilityStateRow,
        r#"
        SELECT sigma_fast_variance, sigma_slow_variance, first_observed_at
        FROM volatility_state
        WHERE pool_address = $1 AND venue = $2 AND timeframe = $3
        "#,
        pool_address,
        venue,
        timeframe,
    )
    .fetch_optional(pool)
    .await
    .wrap_err_with(|| format!("Loading volatility state for {pool_address}/{timeframe}"))?;

    Ok(row)
}
