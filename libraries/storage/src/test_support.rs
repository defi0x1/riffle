// Shared by the `db-tests`-gated integration tests across write:: and queries::. Not compiled
// into a normal `cargo test` run, so it can assume a reachable database without breaking the
// no-database case.

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

// Every sqlx connection is registered with the Tokio I/O driver that was active when it was
// opened, and `#[tokio::test]` hands each test function a private runtime that is torn down
// the instant that test returns. A `PgPool` cached once in a process-wide static and handed
// out to later tests -- as this used to do -- ends up full of connections whose driver no
// longer exists: every query against them hangs until the pool's acquire timeout fires rather
// than failing, which is why the suite got slower and more broken under `--test-threads=1`
// (each test tears down the previous one's runtime before the next reuses the pool) than under
// the default parallel runner (many of those runtimes are still alive when reused). Connecting
// fresh here ties every connection to the same runtime for its entire life, so there is nothing
// left to go stale.
pub async fn test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:55432/feefarming".to_string());

    // Small on purpose: each test opens its own pool now, and dozens of tests running in
    // parallel must not exhaust the server's connection limit between them.
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&database_url)
        .await
        .expect("connecting to test database");

    crate::run_migrations(&pool)
        .await
        .expect("running migrations against test database");

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
        "muted_pools",
        "pool_metrics_5m",
        "indicators_5m",
        "signals",
    ] {
        sqlx::query(&format!("DELETE FROM {table} WHERE pool_address = $1"))
            .bind(pool_address)
            .execute(pool)
            .await
            .unwrap_or_else(|e| panic!("clearing {table} for {pool_address}: {e}"));
    }
}
