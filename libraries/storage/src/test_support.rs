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

// Same reasoning as reset_pool_fixture, for the V2 wallet/position tables: a persistent test
// database sees every previous run's rows, so a test asserting an absolute row count must clear
// its own wallet's tree first. Deleted child-first to respect the FK chain
// (position_cash_flows/position_valuations -> positions -> wallets, transaction_intents ->
// wallets and -> positions).
pub async fn reset_wallet_fixture(pool: &sqlx::PgPool, wallet_address: &str) {
    sqlx::query(
        "DELETE FROM position_cash_flows WHERE position_id IN \
         (SELECT id FROM positions WHERE wallet_address = $1)",
    )
    .bind(wallet_address)
    .execute(pool)
    .await
    .unwrap_or_else(|e| panic!("clearing position_cash_flows for {wallet_address}: {e}"));

    sqlx::query(
        "DELETE FROM position_valuations WHERE position_id IN \
         (SELECT id FROM positions WHERE wallet_address = $1)",
    )
    .bind(wallet_address)
    .execute(pool)
    .await
    .unwrap_or_else(|e| panic!("clearing position_valuations for {wallet_address}: {e}"));

    sqlx::query("DELETE FROM transaction_intents WHERE wallet_address = $1")
        .bind(wallet_address)
        .execute(pool)
        .await
        .unwrap_or_else(|e| panic!("clearing transaction_intents for {wallet_address}: {e}"));

    sqlx::query("DELETE FROM positions WHERE wallet_address = $1")
        .bind(wallet_address)
        .execute(pool)
        .await
        .unwrap_or_else(|e| panic!("clearing positions for {wallet_address}: {e}"));

    sqlx::query("DELETE FROM wallet_balances WHERE wallet_address = $1")
        .bind(wallet_address)
        .execute(pool)
        .await
        .unwrap_or_else(|e| panic!("clearing wallet_balances for {wallet_address}: {e}"));

    sqlx::query("DELETE FROM wallets WHERE pubkey = $1")
        .bind(wallet_address)
        .execute(pool)
        .await
        .unwrap_or_else(|e| panic!("clearing wallets for {wallet_address}: {e}"));
}

// Test-only helper for fixtures that need a pool row to satisfy positions/transaction_intents'
// FK to pools, mirroring the `ensure_pool` helper duplicated across the existing write:: tests.
pub async fn ensure_pool_fixture(pool: &sqlx::PgPool, pool_address: &str) {
    use crate::write::{NewDlmmPoolParams, NewPool, upsert_dlmm_pool};
    use chrono::Utc;
    use rust_decimal::Decimal;

    let now = Utc::now();
    upsert_dlmm_pool(
        pool,
        &NewPool {
            pool_address: pool_address.to_string(),
            venue: crate::types::venue::DLMM,
            token_x: "So11111111111111111111111111111111111111112".to_string(),
            token_y: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
            base_fee_bps: Decimal::new(100, 2),
            protocol_share_bps: 500,
            tvl_usd: None,
            status: 0,
            creator: None,
            activation_point: None,
            created_at: now,
            first_liquidity_at: None,
            is_blacklisted: false,
            launchpad: None,
            tags: vec![],
            updated_at: now,
        },
        &NewDlmmPoolParams {
            pool_address: pool_address.to_string(),
            bin_step: 20,
            base_factor: 10_000,
            filter_period: 30,
            decay_period: 600,
            reduction_factor: 5_000,
            variable_fee_control: 40_000,
            max_volatility_accumulator: 350_000,
            collect_fee_mode: 0,
            reward_mint_x: None,
            reward_mint_y: None,
        },
    )
    .await
    .unwrap();
}
