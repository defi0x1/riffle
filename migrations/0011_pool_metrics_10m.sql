-- Two base resolutions by tier (plans/11 §3, blocking correction R3).
-- Tier-1 pools have a genuine 5-minute base (0010) and this table is a
-- rollup of it. Tier-0 pools on the RPC backend are observed once per
-- 10-minute universe scan -- 10 minutes IS their native resolution, so this
-- table must also accept direct writes for them, not only serve as a
-- rollup target. That dual role is why it is a plain application-managed
-- hypertable rather than a `timescaledb.continuous` view: a continuous
-- aggregate's materialization is refresh-only and cannot mix rows written
-- directly by the application with rows derived from its source.
--
-- `native_resolution = true` marks a row written directly from a 10-minute
-- scan (tier 0); `false` marks a row rolled up from four pool_metrics_5m
-- buckets (tier 1). Downstream consumers use this to know whether a
-- pool's finest data point for this window is genuinely 10 minutes wide
-- or an aggregate of finer samples -- it is also the flag that feeds
-- indicators_{tf}.quality (screening vs measured).
CREATE TABLE pool_metrics_10m (
    pool_address            TEXT NOT NULL REFERENCES pools (pool_address),
    bucket_start             TIMESTAMPTZ NOT NULL,
    native_resolution          BOOLEAN NOT NULL,

    volume_usd              NUMERIC(38,18),
    buy_volume_usd            NUMERIC(38,18),
    sell_volume_usd            NUMERIC(38,18),
    trade_fee_usd             NUMERIC(38,18),
    protocol_fee_usd           NUMERIC(38,18),
    swap_count               INTEGER,
    unique_traders             INTEGER,

    price_open               DOUBLE PRECISION,
    price_high               DOUBLE PRECISION,
    price_low                DOUBLE PRECISION,
    price_close               DOUBLE PRECISION,
    tvl_close                NUMERIC(38,18),
    active_tvl_close           NUMERIC(38,18),
    active_tvl_median           NUMERIC(38,18),
    active_bin_open            INTEGER,
    active_bin_close            INTEGER,
    va_close                 INTEGER,
    total_fee_bps_close          NUMERIC(20,6),
    reserve_x_close             NUMERIC(40,0),
    reserve_y_close             NUMERIC(40,0),

    net_deposit_usd            NUMERIC(38,18),
    add_count                 INTEGER,
    remove_count               INTEGER,
    lp_count_delta              INTEGER,

    PRIMARY KEY (pool_address, bucket_start)
);

SELECT create_hypertable(
    'pool_metrics_10m', 'bucket_start',
    chunk_time_interval => INTERVAL '7 days'
);

CREATE INDEX idx_pool_metrics_10m_bucket ON pool_metrics_10m (bucket_start DESC, pool_address);
