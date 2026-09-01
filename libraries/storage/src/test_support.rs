// Shared by the `db-tests`-gated integration tests across write:: and queries::. Not compiled
// into a normal `cargo test` run, so it can assume a reachable database without breaking the
// no-database case.

use sqlx::PgPool;
use std::sync::OnceLock;

static POOL: OnceLock<PgPool> = OnceLock::new();

pub async fn test_pool() -> PgPool {
    if let Some(pool) = POOL.get() {
        return pool.clone();
    }

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:55432/feefarming".to_string());

    let pool = PgPool::connect(&database_url)
        .await
        .expect("connecting to test database");

    crate::run_migrations(&pool)
        .await
        .expect("running migrations against test database");

    // A OnceLock cannot hold an error path gracefully with set(); this only races on the very
    // first call from concurrent tests, and losing the race just means using the other pool.
    let _ = POOL.set(pool.clone());
    pool
}

// Tests share one database and assert on absolute row counts, so each must start from a known
// state for its own fixture. Without this a second run of the suite sees the first run's rows
// and an idempotency assertion fails for the wrong reason.
pub async fn reset_pool_fixture(pool: &PgPool, pool_address: &str) {
    for table in [
        "swaps",
        "liquidity_events",
        "fee_param_updates",
        "active_bin_snapshots",
        "bin_states",
        "pool_snapshots",
        "dlmm_pool_state",
    ] {
        sqlx::query(&format!("DELETE FROM {table} WHERE pool_address = $1"))
            .bind(pool_address)
            .execute(pool)
            .await
            .unwrap_or_else(|e| panic!("clearing {table} for {pool_address}: {e}"));
    }
}
