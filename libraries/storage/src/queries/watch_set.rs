use chrono::{DateTime, Utc};
use eyre::WrapErr;
use sqlx::PgPool;

use crate::types::tier;

#[derive(Clone, Debug)]
pub struct WatchedPool {
    pub pool_address: String,
    pub venue: i16,
    pub token_x: String,
    pub token_y: String,
    pub bin_step: i16,
    pub tier_changed_at: Option<DateTime<Utc>>,
}

// The tier-1 pools: bin-state subscriptions and quality-A indicators exist only for what this
// returns. The source subsystem diffs its live subscription set against this on every sweep.
pub async fn watch_set(pool: &PgPool) -> eyre::Result<Vec<WatchedPool>> {
    let rows = sqlx::query_as!(
        WatchedPool,
        r#"
        SELECT p.pool_address, p.venue, p.token_x, p.token_y, d.bin_step, p.tier_changed_at
        FROM pools p
        JOIN dlmm_pool_params d ON d.pool_address = p.pool_address
        WHERE p.tier = $1
        ORDER BY p.pool_address
        "#,
        tier::WATCHED,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| "Querying watch set")?;

    Ok(rows)
}
