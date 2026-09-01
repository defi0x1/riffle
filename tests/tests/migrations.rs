//! Property 1: migrations apply cleanly to an empty database, in order, and the resulting
//! schema really carries the hypertables, continuous aggregates, compression and retention
//! policies the migrations claim -- asserted against Timescale's own catalog views, not
//! inferred from the absence of an error. Applying twice must also be a no-op.

use sqlx::Row;

const SCRATCH_DB: &str = "integration_migrations_scratch";

// One row per `create_hypertable(...)` call across the migrations. `indicators_1h/4h/24h`
// are hypertables in their own right (their formulas are not SQL-expressible as a
// continuous aggregate); `pool_metrics_1h/4h/24h` are not in this list because they are
// continuous aggregates instead, asserted separately below.
const EXPECTED_HYPERTABLES: &[&str] = &[
    "swaps",
    "liquidity_events",
    "fee_param_updates",
    "active_bin_snapshots",
    "bin_states",
    "pool_snapshots",
    "dlmm_pool_state",
    "pool_metrics_5m",
    "pool_metrics_10m",
    "indicators_5m",
    "indicators_10m",
    "indicators_1h",
    "indicators_4h",
    "indicators_24h",
    "position_marks",
    "ingest_health",
];

const EXPECTED_CONTINUOUS_AGGREGATES: &[&str] =
    &["pool_metrics_1h", "pool_metrics_4h", "pool_metrics_24h"];

// (hypertable/view name, `compress_after` as Timescale renders the interval).
const EXPECTED_COMPRESSION_POLICIES: &[(&str, &str)] = &[
    ("swaps", "1 day"),
    ("liquidity_events", "7 days"),
    ("fee_param_updates", "30 days"),
    ("active_bin_snapshots", "7 days"),
    ("bin_states", "2 days"),
    ("pool_snapshots", "7 days"),
    ("dlmm_pool_state", "7 days"),
    ("pool_metrics_5m", "30 days"),
    ("pool_metrics_10m", "30 days"),
    ("pool_metrics_1h", "30 days"),
    ("pool_metrics_4h", "30 days"),
    ("pool_metrics_24h", "30 days"),
    ("indicators_5m", "30 days"),
    ("indicators_10m", "30 days"),
    ("indicators_1h", "30 days"),
    ("indicators_4h", "30 days"),
    ("indicators_24h", "30 days"),
    ("position_marks", "30 days"),
    ("ingest_health", "3 days"),
];

// (hypertable name, `drop_after`). Everything not listed here is kept indefinitely by
// design (e.g. `fee_param_updates`, every `pool_metrics_*`/`indicators_*` table,
// `position_marks`) and must NOT have a retention policy.
const EXPECTED_RETENTION_POLICIES: &[(&str, &str)] = &[
    ("swaps", "7 days"),
    ("liquidity_events", "90 days"),
    ("active_bin_snapshots", "90 days"),
    ("bin_states", "14 days"),
    ("pool_snapshots", "90 days"),
    ("dlmm_pool_state", "90 days"),
    ("ingest_health", "30 days"),
];

#[tokio::test]
async fn test_migrations_apply_cleanly_and_the_schema_matches_the_catalog() {
    let Some(base_url) = integration::database_url() else {
        eprintln!(
            "skipping test_migrations_apply_cleanly_and_the_schema_matches_the_catalog: DATABASE_URL is not set"
        );
        return;
    };

    let scratch = integration::fresh_scratch_database(&base_url, SCRATCH_DB).await;

    storage::run_migrations(&scratch)
        .await
        .expect("migrations must apply cleanly to an empty database");

    // Counted from the directory rather than hard-coded, so adding a migration does not fail
    // this test for the wrong reason. What is being asserted is that every file applied, not
    // that there is some particular number of them.
    let on_disk = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../migrations"))
        .expect("reading the migrations directory")
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "sql"))
        .count() as i64;

    let applied_after_first_run: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(&scratch)
        .await
        .expect("counting applied migrations");
    assert_eq!(
        applied_after_first_run, on_disk,
        "every numbered migration on disk must have applied"
    );

    // Applying twice must be safe: sqlx's migrator tracks what it already ran and is a
    // no-op on a second call against the same database.
    storage::run_migrations(&scratch)
        .await
        .expect("re-running migrations against an already-migrated database must be safe");
    let applied_after_second_run: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(&scratch)
        .await
        .expect("counting applied migrations after the second run");
    assert_eq!(
        applied_after_second_run, applied_after_first_run,
        "a second run must not apply anything new"
    );

    assert_hypertables(&scratch).await;
    assert_continuous_aggregates(&scratch).await;
    assert_compression_policies(&scratch).await;
    assert_retention_policies(&scratch).await;
}

async fn assert_hypertables(pool: &sqlx::PgPool) {
    let rows = sqlx::query(
        "SELECT hypertable_name, compression_enabled \
         FROM timescaledb_information.hypertables \
         ORDER BY hypertable_name",
    )
    .fetch_all(pool)
    .await
    .expect("querying timescaledb_information.hypertables");

    let mut found: Vec<String> = rows.iter().map(|r| r.get("hypertable_name")).collect();
    found.sort();
    let mut expected: Vec<String> = EXPECTED_HYPERTABLES.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(
        found, expected,
        "the set of hypertables must match the migrations exactly"
    );

    for row in &rows {
        let name: String = row.get("hypertable_name");
        let compression_enabled: bool = row.get("compression_enabled");
        assert!(
            compression_enabled,
            "hypertable {name} must have compression enabled by migration 0022"
        );
    }
}

async fn assert_continuous_aggregates(pool: &sqlx::PgPool) {
    let rows = sqlx::query(
        "SELECT view_name, materialized_only, compression_enabled \
         FROM timescaledb_information.continuous_aggregates \
         ORDER BY view_name",
    )
    .fetch_all(pool)
    .await
    .expect("querying timescaledb_information.continuous_aggregates");

    let mut found: Vec<String> = rows.iter().map(|r| r.get("view_name")).collect();
    found.sort();
    let mut expected: Vec<String> = EXPECTED_CONTINUOUS_AGGREGATES
        .iter()
        .map(|s| s.to_string())
        .collect();
    expected.sort();
    assert_eq!(
        found, expected,
        "pool_metrics_1h/4h/24h must be the only continuous aggregates"
    );

    for row in &rows {
        let name: String = row.get("view_name");
        let materialized_only: bool = row.get("materialized_only");
        let compression_enabled: bool = row.get("compression_enabled");
        assert!(materialized_only, "{name} must be materialized-only");
        assert!(compression_enabled, "{name} must have compression enabled");
    }
}

async fn assert_compression_policies(pool: &sqlx::PgPool) {
    let rows = sqlx::query(
        "SELECT hypertable_name, config ->> 'compress_after' AS compress_after \
         FROM timescaledb_information.jobs \
         WHERE proc_name = 'policy_compression'",
    )
    .fetch_all(pool)
    .await
    .expect("querying compression policy jobs");

    for (table, expected_interval) in EXPECTED_COMPRESSION_POLICIES {
        let matched = rows
            .iter()
            .find(|r| r.get::<String, _>("hypertable_name") == *table);
        let row = matched.unwrap_or_else(|| panic!("no compression policy found for {table}"));
        let observed: String = row.get("compress_after");
        assert_eq!(
            &observed, expected_interval,
            "compression policy for {table} has the wrong compress_after"
        );
    }
    assert_eq!(
        rows.len(),
        EXPECTED_COMPRESSION_POLICIES.len(),
        "unexpected number of compression policies -- a table gained or lost one"
    );
}

async fn assert_retention_policies(pool: &sqlx::PgPool) {
    let rows = sqlx::query(
        "SELECT hypertable_name, config ->> 'drop_after' AS drop_after \
         FROM timescaledb_information.jobs \
         WHERE proc_name = 'policy_retention'",
    )
    .fetch_all(pool)
    .await
    .expect("querying retention policy jobs");

    for (table, expected_interval) in EXPECTED_RETENTION_POLICIES {
        let matched = rows
            .iter()
            .find(|r| r.get::<String, _>("hypertable_name") == *table);
        let row = matched.unwrap_or_else(|| panic!("no retention policy found for {table}"));
        let observed: String = row.get("drop_after");
        assert_eq!(
            &observed, expected_interval,
            "retention policy for {table} has the wrong drop_after"
        );
    }
    assert_eq!(
        rows.len(),
        EXPECTED_RETENTION_POLICIES.len(),
        "unexpected number of retention policies -- a table gained or lost one, e.g. \
         fee_param_updates/pool_metrics_*/indicators_*/position_marks must never get one"
    );
}
