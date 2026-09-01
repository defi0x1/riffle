// Shared by the `db-tests`-gated integration tests across write:: and queries::. Not compiled
// into a normal `cargo test` run, so it can assume a reachable database without breaking the
// no-database case.
#![cfg(feature = "db-tests")]

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
