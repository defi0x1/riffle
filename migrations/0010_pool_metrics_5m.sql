-- The base of the rollup layer. Two halves per bucket: flow
-- (summed from swaps and liquidity_events) and state (sampled from
-- pool_snapshots). Storing both together per bucket is what makes every
-- indicator computable at every timeframe from one table.

-- NOTE: this is a plain hypertable, populated by the application on a
-- 5-minute tick, not a `CREATE MATERIALIZED VIEW... WITH
-- (timescaledb.continuous)`. TimescaleDB continuous aggregates support only
-- one hypertable in their FROM clause (joins to a hypertable and a plain
-- table are fine; two hypertables are not), and this table's own definition
-- needs three: swaps, liquidity_events and pool_snapshots. A native CAGG
-- was not an option here, only a choice between hand-rolling this table or
-- building three narrower single-source CAGGs and joining them at query
-- time on every read; the latter pushes the join onto the hot query path
-- for no benefit, since the write-time join happens once per bucket either
-- way. pool_metrics_10m/1h/4h/24h (0011-0014) do not have this constraint
-- and are real hierarchical continuous aggregates.

-- A pool with no genuine 5-minute observation (tier-0 on the RPC backend,
-- see 0011) simply has no row for that bucket. Absence is honest; a
-- fabricated zero is not.
CREATE TABLE pool_metrics_5m (
    pool_address            TEXT NOT NULL REFERENCES pools (pool_address),
    bucket_start             TIMESTAMPTZ NOT NULL,

    -- flow, summed over the bucket from swaps
    volume_usd              NUMERIC(38,18),
    buy_volume_usd            NUMERIC(38,18),
    sell_volume_usd            NUMERIC(38,18),
    trade_fee_usd             NUMERIC(38,18),
    protocol_fee_usd           NUMERIC(38,18),
    swap_count               INTEGER,
    unique_traders             INTEGER,

    -- state, sampled from pool_snapshots
    price_open               DOUBLE PRECISION,
    price_high               DOUBLE PRECISION,
    price_low                DOUBLE PRECISION,
    price_close               DOUBLE PRECISION,
    tvl_close                NUMERIC(38,18),
    active_tvl_close           NUMERIC(38,18),
    -- L-bar_a: median over the trailing 60 minutes, from active_bin_snapshots.
    active_tvl_median           NUMERIC(38,18),
    active_bin_open            INTEGER,
    active_bin_close            INTEGER,
    va_close                 INTEGER,
    total_fee_bps_close          NUMERIC(20,6),
    reserve_x_close             NUMERIC(40,0),
    reserve_y_close             NUMERIC(40,0),

    -- liquidity flow, from liquidity_events
    net_deposit_usd            NUMERIC(38,18),
    add_count                 INTEGER,
    remove_count               INTEGER,
    lp_count_delta              INTEGER,

    PRIMARY KEY (pool_address, bucket_start)
);

-- Modest volume (~30k rows/day across all pools,) and indefinite
-- retention: a wide chunk keeps the chunk count low over a multi-year
-- lifetime.
SELECT create_hypertable(
    'pool_metrics_5m', 'bucket_start',
    chunk_time_interval => INTERVAL '7 days'
);

CREATE INDEX idx_pool_metrics_5m_bucket ON pool_metrics_5m (bucket_start DESC, pool_address);
