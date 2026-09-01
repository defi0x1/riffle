use chrono::{DateTime, Utc};
use eyre::WrapErr;
use sqlx::PgPool;

#[derive(Clone, Debug)]
pub struct NewRegimeStateRow {
    pub pool_address: String,
    pub venue: i16,
    pub timeframe: String,
    pub regime: Option<String>,
    pub since: DateTime<Utc>,
    pub pending: Option<String>,
    pub pending_since: Option<DateTime<Utc>>,
    pub last_transition: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

// One row per (pool, venue, timeframe); a tick always has a full RegimeState to save, so this
// is a plain upsert rather than a batch -- the pipeline evaluates one pool/timeframe at a time.
pub async fn upsert_regime_state(pool: &PgPool, row: &NewRegimeStateRow) -> eyre::Result<()> {
    sqlx::query!(
        r#"
        INSERT INTO regime_state (
            pool_address, venue, timeframe, regime, since, pending, pending_since,
            last_transition, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        ON CONFLICT (pool_address, venue, timeframe) DO UPDATE SET
            regime          = EXCLUDED.regime,
            since           = EXCLUDED.since,
            pending         = EXCLUDED.pending,
            pending_since   = EXCLUDED.pending_since,
            last_transition = EXCLUDED.last_transition,
            updated_at      = EXCLUDED.updated_at
        "#,
        row.pool_address,
        row.venue,
        row.timeframe,
        row.regime,
        row.since,
        row.pending,
        row.pending_since,
        row.last_transition,
        row.updated_at,
    )
    .execute(pool)
    .await
    .wrap_err_with(|| {
        format!(
            "Upserting regime state for {}/{}",
            row.pool_address, row.timeframe
        )
    })?;

    Ok(())
}

#[derive(Clone, Debug)]
pub struct NewVolatilityStateRow {
    pub pool_address: String,
    pub venue: i16,
    pub timeframe: String,
    pub sigma_fast_variance: f64,
    pub sigma_slow_variance: f64,
    pub first_observed_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn upsert_volatility_state(
    pool: &PgPool,
    row: &NewVolatilityStateRow,
) -> eyre::Result<()> {
    sqlx::query!(
        r#"
        INSERT INTO volatility_state (
            pool_address, venue, timeframe, sigma_fast_variance, sigma_slow_variance,
            first_observed_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (pool_address, venue, timeframe) DO UPDATE SET
            sigma_fast_variance = EXCLUDED.sigma_fast_variance,
            sigma_slow_variance = EXCLUDED.sigma_slow_variance,
            updated_at          = EXCLUDED.updated_at
        "#,
        row.pool_address,
        row.venue,
        row.timeframe,
        row.sigma_fast_variance,
        row.sigma_slow_variance,
        row.first_observed_at,
        row.updated_at,
    )
    .execute(pool)
    .await
    .wrap_err_with(|| {
        format!(
            "Upserting volatility state for {}/{}",
            row.pool_address, row.timeframe
        )
    })?;

    Ok(())
}
