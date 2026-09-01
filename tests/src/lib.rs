//! Shared fixtures for the end-to-end integration suite. Every test in `tests/tests/`
//! gates itself on [`shared_pool`] and returns early when it comes back `None`, so
//! `cargo test --workspace` still passes with no database configured -- the suite proves
//! nothing in that case, but it does not fail the build either.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use storage::types::venue;
use storage::write::{NewDlmmPoolParams, NewPool, upsert_dlmm_pool};

/// `None` when unset or empty, never a guessed fallback -- the whole point is a clean skip,
/// not a silent connect attempt against a URL nobody configured.
pub fn database_url() -> Option<String> {
    std::env::var("DATABASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
}

/// A pool for the calling test, or `None` if `DATABASE_URL` is unset.
///
/// Deliberately NOT a single pool shared process-wide behind a `OnceLock`: `#[tokio::test]`
/// gives every test function its own throwaway single-threaded Tokio runtime, and a
/// `sqlx::PgPool` created inside one such runtime is not safe to keep using once that
/// runtime shuts down -- with many tests in one binary running in parallel (the default),
/// sharing one pool across them leaked connection permits under real load (observed as
/// either a "Tokio 1.x context ... is being shutdown" panic or every later caller hanging
/// until `acquire_timeout` with the server itself nowhere near its own connection limit).
/// A small pool built fresh per test lives and dies entirely within that one test's own
/// runtime, which sidesteps the whole class of bug. Re-running migrations per test is cheap
/// (`sqlx::migrate::Migrator::run` is a handful of round trips once everything is already
/// applied) and keeps this identical in behaviour to the shared-pool version for callers.
pub async fn shared_pool() -> Option<PgPool> {
    let url = database_url()?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(30))
        .connect(&url)
        .await
        .unwrap_or_else(|e| panic!("connecting to integration test database: {e}"));
    storage::run_migrations(&pool)
        .await
        .unwrap_or_else(|e| panic!("running migrations against integration test database: {e}"));
    Some(pool)
}

/// Skip this test cleanly and say why, rather than failing the build on a machine with no
/// database configured. Usage: `let pool = require_database!();` at the top of a test.
#[macro_export]
macro_rules! require_database {
    () => {
        match $crate::shared_pool().await {
            Some(pool) => pool,
            None => {
                eprintln!("skipping {}: DATABASE_URL is not set", module_path!());
                return;
            }
        }
    };
}

/// Deletes every row this suite could have left behind for `pool_address`, so a rerun
/// against the same persistent database starts from a known state instead of tripping an
/// idempotency assertion on stale rows from a previous run. Mirrors
/// `storage::test_support::reset_pool_fixture` (private to that crate) plus the extra
/// tables this suite also writes to. `outcomes`/`position_marks` key on `paper_positions.id`
/// and `rationale` keys on `signals.id`, not on `pool_address` directly, and both parents
/// have a plain (non-cascading) foreign key from their children, so those go first or the
/// later `DELETE FROM paper_positions`/`signals` would fail with a foreign-key violation on
/// a second run of the suite.
pub async fn reset_pool_fixture(pool: &PgPool, pool_address: &str) {
    // One round trip, not two dozen: with every `tests/tests/*.rs` file's tests running in
    // parallel by default and each calling this at least once, a chatty per-table sequence
    // of round trips was serializing on connection acquisition under load. `pool_address` is
    // always a static test literal (never external input), so interpolating it into one
    // statement -- doubling any embedded quote defensively even though none of this suite's
    // fixtures ever contain one -- is safe here the way it already is for the scratch
    // database name below.
    let escaped = pool_address.replace('\'', "''");
    let sql = format!(
        "DELETE FROM outcomes WHERE position_id IN (SELECT id FROM paper_positions WHERE pool_address = '{escaped}');
         DELETE FROM position_marks WHERE position_id IN (SELECT id FROM paper_positions WHERE pool_address = '{escaped}');
         DELETE FROM paper_positions WHERE pool_address = '{escaped}';
         DELETE FROM rationale WHERE signal_id IN (SELECT id FROM signals WHERE pool_address = '{escaped}');
         DELETE FROM signals WHERE pool_address = '{escaped}';
         DELETE FROM swaps WHERE pool_address = '{escaped}';
         DELETE FROM liquidity_events WHERE pool_address = '{escaped}';
         DELETE FROM fee_param_updates WHERE pool_address = '{escaped}';
         DELETE FROM active_bin_snapshots WHERE pool_address = '{escaped}';
         DELETE FROM bin_states WHERE pool_address = '{escaped}';
         DELETE FROM pool_snapshots WHERE pool_address = '{escaped}';
         DELETE FROM dlmm_pool_state WHERE pool_address = '{escaped}';
         DELETE FROM pool_metrics_5m WHERE pool_address = '{escaped}';
         DELETE FROM pool_metrics_10m WHERE pool_address = '{escaped}';
         DELETE FROM indicators_5m WHERE pool_address = '{escaped}';
         DELETE FROM indicators_10m WHERE pool_address = '{escaped}';
         DELETE FROM indicators_1h WHERE pool_address = '{escaped}';
         DELETE FROM indicators_4h WHERE pool_address = '{escaped}';
         DELETE FROM indicators_24h WHERE pool_address = '{escaped}';
         DELETE FROM regime_state WHERE pool_address = '{escaped}';
         DELETE FROM volatility_state WHERE pool_address = '{escaped}';
         DELETE FROM muted_pools WHERE pool_address = '{escaped}';"
    );
    sqlx::raw_sql(&sql)
        .execute(pool)
        .await
        .unwrap_or_else(|e| panic!("clearing fixture rows for {pool_address}: {e}"));
}

/// A base timestamp fixtures anchor to, fixed rather than `Utc::now()`: several tables key
/// idempotency on `ts`, so a deterministic value makes replay assertions hold across
/// repeated runs against a persistent database, not only within one process.
pub fn fixture_time() -> DateTime<Utc> {
    "2026-01-01T00:00:00Z".parse().unwrap()
}

/// Swaps the database name out of a `postgres://.../<db>` URL for `postgres`, the
/// maintenance database every server has, so `CREATE DATABASE`/`DROP DATABASE` have
/// somewhere to run from (Postgres refuses to touch the database a connection is on).
fn maintenance_url(database_url: &str) -> String {
    let idx = database_url
        .rfind('/')
        .expect("DATABASE_URL must contain a path");
    format!("{}/postgres", &database_url[..idx])
}

fn with_database_name(database_url: &str, db_name: &str) -> String {
    let idx = database_url
        .rfind('/')
        .expect("DATABASE_URL must contain a path");
    format!("{}/{}", &database_url[..idx], db_name)
}

/// Drops and recreates `db_name` on the same server `database_url` points at, then returns
/// a pool connected to the fresh, empty database. `db_name` is a fixed literal the caller
/// controls (never external input), so building the DDL by format string is safe here --
/// Postgres has no bind-parameter form for an identifier in `CREATE`/`DROP DATABASE`.
pub async fn fresh_scratch_database(database_url: &str, db_name: &str) -> PgPool {
    let admin_pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&maintenance_url(database_url))
        .await
        .unwrap_or_else(|e| panic!("connecting to the maintenance database: {e}"));

    sqlx::query(&format!(
        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
         WHERE datname = '{db_name}' AND pid <> pg_backend_pid()"
    ))
    .execute(&admin_pool)
    .await
    .unwrap_or_else(|e| panic!("terminating leftover connections to {db_name}: {e}"));

    sqlx::query(&format!("DROP DATABASE IF EXISTS {db_name}"))
        .execute(&admin_pool)
        .await
        .unwrap_or_else(|e| panic!("dropping {db_name}: {e}"));

    sqlx::query(&format!("CREATE DATABASE {db_name}"))
        .execute(&admin_pool)
        .await
        .unwrap_or_else(|e| panic!("creating {db_name}: {e}"));

    admin_pool.close().await;

    PgPoolOptions::new()
        .max_connections(5)
        .connect(&with_database_name(database_url, db_name))
        .await
        .unwrap_or_else(|e| panic!("connecting to fresh scratch database {db_name}: {e}"))
}

pub const WRAPPED_SOL: &str = "So11111111111111111111111111111111111111112";
pub const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

/// A standard DLMM pool + params pair for `pool_address`, built but not written -- callers
/// that need to mutate a field before writing (or that only need the shape, e.g. to compare
/// against a query result) use this directly; [`ensure_pool`]/[`ensure_pool_with`] are the
/// write-it-for-me convenience on top.
pub fn sample_pool(pool_address: &str) -> (NewPool, NewDlmmPoolParams) {
    let now = fixture_time();
    (
        NewPool {
            pool_address: pool_address.to_string(),
            venue: venue::DLMM,
            token_x: WRAPPED_SOL.to_string(),
            token_y: USDC.to_string(),
            base_fee_bps: Decimal::new(100, 2),
            protocol_share_bps: 500,
            tvl_usd: Some(Decimal::new(5_000_000, 0)),
            status: 0,
            creator: None,
            activation_point: None,
            created_at: now,
            first_liquidity_at: Some(now),
            is_blacklisted: false,
            launchpad: None,
            tags: vec![],
            updated_at: now,
        },
        NewDlmmPoolParams {
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
}

/// A standard DLMM pool + params pair, upserted for `pool_address`. `upsert_dlmm_pool` is
/// an upsert, so calling this again for the same address is safe and just refreshes it.
pub async fn ensure_pool(pool: &PgPool, pool_address: &str) {
    ensure_pool_with(pool, pool_address, |_, _| {}).await;
}

/// Like [`ensure_pool`], but lets the caller adjust the shared/DLMM rows before they are
/// written -- for scenarios (e.g. risk-gate rejection) that need a non-default field.
pub async fn ensure_pool_with(
    pool: &PgPool,
    pool_address: &str,
    edit: impl FnOnce(&mut NewPool, &mut NewDlmmPoolParams),
) {
    let (mut shared, mut params) = sample_pool(pool_address);
    edit(&mut shared, &mut params);
    upsert_dlmm_pool(pool, &shared, &params)
        .await
        .unwrap_or_else(|e| panic!("upserting fixture pool {pool_address}: {e:?}"));
}

/// `pool_metrics_1h`/`4h`/`24h` are `WITH NO DATA` continuous aggregates (migrations
/// 0012-0014): nothing appears in them until either Timescale's own background refresh
/// policy runs or something calls this. Production never calls it directly -- the policy job
/// does -- so this is the one piece of the rollup-chain test with no production equivalent to
/// call through; it drives the same procedure the policy job drives, over the view's whole
/// range, for a deterministic result instead of waiting on a schedule.
pub async fn refresh_continuous_aggregate(pool: &PgPool, view: &str) {
    sqlx::query(&format!(
        "CALL refresh_continuous_aggregate('{view}', NULL, NULL)"
    ))
    .execute(pool)
    .await
    .unwrap_or_else(|e| panic!("refreshing continuous aggregate {view}: {e}"));
}
