//! Property 6: restart safety. Regime hysteresis and volatility state are the only worker
//! state this codebase persists to survive a restart (migration `0023_scorer_worker_state`,
//! tables `regime_state`/`volatility_state`; `bin/scorer/src/state.rs` converts between
//! these rows and `engine`'s in-memory `RegimeState`/`VolatilityState`). Write state, open a
//! brand-new connection pool (not the shared fixture pool -- the point is a fresh handle,
//! the way a restarted process gets one), read it back, and assert the hysteresis fields
//! (`pending`, `pending_since`, `last_transition`) and volatility variances survive exactly.
//!
//! Note on scope: the indexer's own progress tracking (`bin/indexer/src/workers/progress.rs`,
//! `Progress`) is in-memory atomics only -- there is no persisted ingestion cursor/watermark
//! table anywhere in the schema (`0023` is the only "worker state" migration, and it holds
//! only regime/volatility state). Restart safety for ingestion instead comes from the raw
//! event tables' idempotent `ON CONFLICT ... DO NOTHING` writes (proved in
//! `idempotency.rs`): a restarted indexer safely re-scans rather than resuming from a saved
//! position. This test cannot prove a cursor survives a restart because production does not
//! have one to prove; see the suite's summary report.

use engine::Regime;
use engine::regime::RegimeState;
use engine::volatility::VolatilityState;
use scorer::state::{
    regime_state_from_row, regime_state_to_row, volatility_state_from_row, volatility_state_to_row,
};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use storage::queries::{load_regime_state, load_volatility_state};

async fn fresh_connection() -> PgPool {
    let database_url = integration::database_url().expect("checked by require_database! above");
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .unwrap_or_else(|e| panic!("opening a fresh connection pool: {e}"))
}

#[tokio::test]
async fn test_regime_hysteresis_survives_a_restart() {
    let pool = integration::require_database!();
    let pool_address = "restart_safety_regime";
    integration::ensure_pool(&pool, pool_address).await;
    integration::reset_pool_fixture(&pool, pool_address).await;

    let venue = storage::types::venue::DLMM;
    let timeframe = "1h";
    let now = integration::fixture_time();

    // A regime mid-hysteresis: committed to V1, but with a V2 candidate pending since 12
    // minutes ago and one prior transition recorded. This is exactly the state a restart
    // must not lose -- losing `pending`/`pending_since` resets the persistence clock and
    // the classifier has to start the whole window over.
    let before = RegimeState {
        regime: Some(Regime::V1),
        since: now - chrono::Duration::days(3),
        pending: Some(Regime::V2),
        pending_since: Some(now - chrono::Duration::minutes(12)),
        last_transition: Some(now - chrono::Duration::days(3)),
    };
    let row = regime_state_to_row(pool_address, venue, timeframe, &before, now);
    storage::write::upsert_regime_state(&pool, &row)
        .await
        .expect("persisting regime state");

    // Drop every handle this test has held so far and open one that has never seen this
    // process's in-memory state -- the same thing a restarted worker does.
    drop(pool);
    let restarted = fresh_connection().await;

    let loaded = load_regime_state(&restarted, pool_address, venue, timeframe)
        .await
        .expect("loading regime state after restart")
        .expect("regime state row must still exist after restart");
    let after = regime_state_from_row(&loaded);

    assert_eq!(after.regime, before.regime);
    assert_eq!(after.since, before.since);
    assert_eq!(after.pending, before.pending);
    assert_eq!(after.pending_since, before.pending_since);
    assert_eq!(after.last_transition, before.last_transition);
}

#[tokio::test]
async fn test_volatility_state_survives_a_restart() {
    let pool = integration::require_database!();
    let pool_address = "restart_safety_volatility";
    integration::ensure_pool(&pool, pool_address).await;
    integration::reset_pool_fixture(&pool, pool_address).await;

    let venue = storage::types::venue::DLMM;
    let timeframe = "1h";
    let now = integration::fixture_time();

    let before = VolatilityState {
        sigma_fast_variance: 0.000_123_456_789,
        sigma_slow_variance: 0.000_045_678_901,
        first_observed_at: now - chrono::Duration::days(30),
    };
    let row = volatility_state_to_row(pool_address, venue, timeframe, &before, now);
    storage::write::upsert_volatility_state(&pool, &row)
        .await
        .expect("persisting volatility state");

    drop(pool);
    let restarted = fresh_connection().await;

    let loaded = load_volatility_state(&restarted, pool_address, venue, timeframe)
        .await
        .expect("loading volatility state after restart")
        .expect("volatility state row must still exist after restart");
    let after = volatility_state_from_row(&loaded);

    assert_eq!(after.sigma_fast_variance, before.sigma_fast_variance);
    assert_eq!(after.sigma_slow_variance, before.sigma_slow_variance);
    assert_eq!(after.first_observed_at, before.first_observed_at);
}

#[tokio::test]
async fn test_regime_state_upsert_advances_hysteresis_across_a_restart() {
    // A restart is not just "read what was there" -- it is "read what was there, then keep
    // ticking". Write an initial pending-candidate state, restart, then write the next
    // tick's state (the candidate has now persisted long enough to commit) through the
    // fresh connection, and confirm a *third* connection sees the fully-advanced state --
    // proving the row a restarted worker resumes from is the row it can also keep updating,
    // not a read-only snapshot.
    let pool = integration::require_database!();
    let pool_address = "restart_safety_regime_advance";
    integration::ensure_pool(&pool, pool_address).await;
    integration::reset_pool_fixture(&pool, pool_address).await;

    let venue = storage::types::venue::DLMM;
    let timeframe = "1h";
    let now = integration::fixture_time();

    let pending_state = RegimeState {
        regime: Some(Regime::V1),
        since: now - chrono::Duration::days(1),
        pending: Some(Regime::V2),
        pending_since: Some(now - chrono::Duration::minutes(29)),
        last_transition: Some(now - chrono::Duration::days(1)),
    };
    storage::write::upsert_regime_state(
        &pool,
        &regime_state_to_row(pool_address, venue, timeframe, &pending_state, now),
    )
    .await
    .unwrap();
    drop(pool);

    let after_restart = fresh_connection().await;
    let resumed = regime_state_from_row(
        &load_regime_state(&after_restart, pool_address, venue, timeframe)
            .await
            .unwrap()
            .unwrap(),
    );
    assert_eq!(resumed.pending, Some(Regime::V2));

    let committed_now = now + chrono::Duration::minutes(2);
    let committed_state = RegimeState {
        regime: Some(Regime::V2),
        since: committed_now,
        pending: None,
        pending_since: None,
        last_transition: Some(committed_now),
    };
    storage::write::upsert_regime_state(
        &after_restart,
        &regime_state_to_row(
            pool_address,
            venue,
            timeframe,
            &committed_state,
            committed_now,
        ),
    )
    .await
    .unwrap();
    drop(after_restart);

    let observer = fresh_connection().await;
    let final_state = regime_state_from_row(
        &load_regime_state(&observer, pool_address, venue, timeframe)
            .await
            .unwrap()
            .unwrap(),
    );
    assert_eq!(final_state.regime, Some(Regime::V2));
    assert_eq!(final_state.pending, None);
    assert_eq!(final_state.since, committed_now);
}
