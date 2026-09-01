//! Shared by the `db-tests`-gated tests across this crate, mirroring storage's own
//! `test_support` module (not reusable directly: it is a private, test-only module internal to
//! the `storage` crate). Not compiled into a normal `cargo test` run, so it can assume a
//! reachable database without breaking the no-database case.

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

use crate::config::Args;
use crate::state::AppState;

pub async fn test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://feefarm:feefarm@localhost:5432/feefarm".to_string());

    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&database_url)
        .await
        .expect("connecting to test database");

    storage::run_migrations(&pool)
        .await
        .expect("running migrations against test database");

    pool
}

/// A minimal `AppState` for tests that only ever touch `state.db` -- `risk`, `wallet_resolve`
/// and `tx_build::create_intent_idempotently` never read `state.rpc` or most of `state.config`.
/// The RPC client points nowhere reachable on purpose: constructing one does not itself connect,
/// and a test that accidentally needed a real RPC call would fail loudly rather than silently
/// hitting a real network.
pub fn test_state(db: PgPool) -> AppState {
    let args = Args {
        logging: logger::Config {
            log_level: "error".to_string(),
            log_format: logger::LogFormat::Compact,
        },
        postgres: common::PostgresConfig {
            database_url: "unused".to_string(),
            max_connections: 1,
        },
        metrics: metrics::Config {
            disable_metrics_server: true,
            metrics_port: 0,
        },
        bot_token: "test-bot-token".to_string(),
        rpc_url: "http://127.0.0.1:1".to_string(),
        port: 0,
        init_data_max_age: Duration::from_secs(86_400),
        max_amount_raw: 1_000_000_000,
        compute_unit_limit: 200_000,
        compute_unit_price_micro_lamports: 0,
        intent_expiry: Duration::from_secs(60),
        confirmation_timeout: Duration::from_secs(1),
        confirmation_poll_interval: Duration::from_millis(10),
        config: None,
    };

    AppState {
        db,
        rpc: Arc::new(solana_rpc_client::nonblocking::rpc_client::RpcClient::new(
            args.rpc_url.clone(),
        )),
        config: Arc::new(args),
    }
}
