-- Retention and compression policies for every hypertable, collected here
-- as one concern rather than scattered across each table's own migration.
-- The rule throughout: raw data expires, aggregates and decisions are
-- kept forever -- rebuilding an
-- aggregate from expired raw is impossible, so the aggregates are the
-- durable artefact.

-- swaps: whole-program flow on the Geyser backend, the dominant unknown
-- in the capacity budget. Retention starts at 7 days, not
-- 90, until 24 hours of measured volume justifies extending it -- sizing
-- this table from a guess is how a disk fills at 3 a.m. Compression
-- window is shortened to match: compressing at the original 7-day mark
-- would land right as chunks are being dropped, so it buys nothing at
-- this retention.
-- orderby carries the rest of the primary key (signature, ix_index) so
-- compression has a fully deterministic row order within a segment.
ALTER TABLE swaps SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'pool_address',
    timescaledb.compress_orderby = 'ts DESC, signature, ix_index'
);
SELECT add_compression_policy('swaps', INTERVAL '1 day');
SELECT add_retention_policy('swaps', INTERVAL '7 days');

-- liquidity_events: low volume regardless of backend; the 90-day figure.
ALTER TABLE liquidity_events SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'pool_address',
    timescaledb.compress_orderby = 'ts DESC, signature, ix_index'
);
SELECT add_compression_policy('liquidity_events', INTERVAL '7 days');
SELECT add_retention_policy('liquidity_events', INTERVAL '90 days');

-- fee_param_updates: rare events (creator fee-parameter changes only) and
-- an input to an instant-exit / kill-condition check, so kept indefinitely
-- like the decision-evidence tables rather than the 90-day raw default --
-- no retention policy is added. Still compressed once old, since it is
-- rarely queried past its first month.
ALTER TABLE fee_param_updates SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'pool_address',
    timescaledb.compress_orderby = 'ts DESC, signature, field'
);
SELECT add_compression_policy('fee_param_updates', INTERVAL '30 days');

-- active_bin_snapshots:.
ALTER TABLE active_bin_snapshots SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'pool_address',
    timescaledb.compress_orderby = 'ts DESC'
);
SELECT add_compression_policy('active_bin_snapshots', INTERVAL '7 days');
SELECT add_retention_policy('active_bin_snapshots', INTERVAL '90 days');

-- bin_states: 14-day retention, compression after 2 days ( --
-- the ~100x volume correction that drove the active_bin_snapshots split).
ALTER TABLE bin_states SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'pool_address',
    timescaledb.compress_orderby = 'bin_id, ts DESC'
);
SELECT add_compression_policy('bin_states', INTERVAL '2 days');
SELECT add_retention_policy('bin_states', INTERVAL '14 days');

-- pool_snapshots / dlmm_pool_state: same table, split by the shared/
-- satellite rule (0009), so the same retention and compression schedule.
ALTER TABLE pool_snapshots SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'pool_address',
    timescaledb.compress_orderby = 'ts DESC'
);
SELECT add_compression_policy('pool_snapshots', INTERVAL '7 days');
SELECT add_retention_policy('pool_snapshots', INTERVAL '90 days');

ALTER TABLE dlmm_pool_state SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'pool_address',
    timescaledb.compress_orderby = 'ts DESC'
);
SELECT add_compression_policy('dlmm_pool_state', INTERVAL '7 days');
SELECT add_retention_policy('dlmm_pool_state', INTERVAL '90 days');

-- pool_metrics_{5m,10m}: the durable long-term record;
-- indefinite retention, compressed after 30 days. Plain application-
-- managed hypertables (see 0010/0011 for why they are not native
-- continuous aggregates), so they take the same compression treatment as
-- any other hypertable rather than a continuous-aggregate policy.
ALTER TABLE pool_metrics_5m SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'pool_address',
    timescaledb.compress_orderby = 'bucket_start DESC'
);
SELECT add_compression_policy('pool_metrics_5m', INTERVAL '30 days');

ALTER TABLE pool_metrics_10m SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'pool_address',
    timescaledb.compress_orderby = 'bucket_start DESC'
);
SELECT add_compression_policy('pool_metrics_10m', INTERVAL '30 days');

-- pool_metrics_{1h,4h,24h}: real continuous aggregates (0012-0014); their
-- materialization hypertable takes the same compression window for
-- consistency with the rest of the durable record.
ALTER MATERIALIZED VIEW pool_metrics_1h SET (
    timescaledb.compress = true,
    timescaledb.compress_segmentby = 'pool_address',
    timescaledb.compress_orderby = 'bucket_start DESC'
);
SELECT add_compression_policy('pool_metrics_1h', INTERVAL '30 days');

ALTER MATERIALIZED VIEW pool_metrics_4h SET (
    timescaledb.compress = true,
    timescaledb.compress_segmentby = 'pool_address',
    timescaledb.compress_orderby = 'bucket_start DESC'
);
SELECT add_compression_policy('pool_metrics_4h', INTERVAL '30 days');

ALTER MATERIALIZED VIEW pool_metrics_24h SET (
    timescaledb.compress = true,
    timescaledb.compress_segmentby = 'pool_address',
    timescaledb.compress_orderby = 'bucket_start DESC'
);
SELECT add_compression_policy('pool_metrics_24h', INTERVAL '30 days');

-- indicators_{5m,10m,1h,4h,24h}: same treatment. All five
-- are application-managed hypertables (0015), not continuous aggregates.
ALTER TABLE indicators_5m SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'pool_address',
    timescaledb.compress_orderby = 'bucket_start DESC'
);
SELECT add_compression_policy('indicators_5m', INTERVAL '30 days');

ALTER TABLE indicators_10m SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'pool_address',
    timescaledb.compress_orderby = 'bucket_start DESC'
);
SELECT add_compression_policy('indicators_10m', INTERVAL '30 days');

ALTER TABLE indicators_1h SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'pool_address',
    timescaledb.compress_orderby = 'bucket_start DESC'
);
SELECT add_compression_policy('indicators_1h', INTERVAL '30 days');

ALTER TABLE indicators_4h SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'pool_address',
    timescaledb.compress_orderby = 'bucket_start DESC'
);
SELECT add_compression_policy('indicators_4h', INTERVAL '30 days');

ALTER TABLE indicators_24h SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'pool_address',
    timescaledb.compress_orderby = 'bucket_start DESC'
);
SELECT add_compression_policy('indicators_24h', INTERVAL '30 days');

-- position_marks: not named in's table, but it is the
-- evidence outcomes are scored against, so it gets the same "keep
-- forever, compress once cold" treatment as the rest of the derived
-- layer rather than the 90-day raw-event default.
ALTER TABLE position_marks SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'position_id',
    timescaledb.compress_orderby = 'ts DESC'
);
SELECT add_compression_policy('position_marks', INTERVAL '30 days');

-- ingest_health: operational telemetry, not evidence -- Prometheus is the
-- durable store for this, so Postgres keeps only enough for
-- /status to answer recent-history questions.
ALTER TABLE ingest_health SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'source',
    timescaledb.compress_orderby = 'ts DESC'
);
SELECT add_compression_policy('ingest_health', INTERVAL '3 days');
SELECT add_retention_policy('ingest_health', INTERVAL '30 days');
