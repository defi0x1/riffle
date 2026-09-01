-- Derived layer: one row per pool per bucket per
-- timeframe, computed by the application from pool_metrics_{tf} and
-- written directly -- these are not database continuous aggregates, since
-- the indicator formulas (volatility estimators, the organic-flow blend,
-- the ranking metric) are not expressible as SQL aggregate functions.

-- `venue` is added here (and on signals, rationale, paper_positions,
-- outcomes): the ranking metrics reduce to one shared
-- expression across venues, so the only thing these tables need to grow
-- for a second venue is rows with a different `venue` value.

-- *_change columns are the previous bucket's comparable value, computed by
-- the application at write time (a plain LAG against the last row for the
-- pool) and stored rather than read back with a window function -- this is
-- the single most-read comparison in the system (every /pool_detail and
-- /potential render), so it is paid for once at write time, not on every
-- read.
CREATE TABLE indicators_5m (
    pool_address       TEXT NOT NULL REFERENCES pools (pool_address),
    venue              SMALLINT NOT NULL,
    bucket_start        TIMESTAMPTZ NOT NULL,

    -- A = measured L-bar_a (tier 1). B = TVL x phi_shape estimate (tier 0).
    -- Only A counts toward outcome scoring. Fixed two-valued
    -- semantic, not a venue-scoped enum, so a CHECK is appropriate here.
    quality            CHAR(1) NOT NULL CHECK (quality IN ('A', 'B')),
    regime             TEXT,

    vol_change          DOUBLE PRECISION,
    fee_change          DOUBLE PRECISION,
    tvl_change          DOUBLE PRECISION,
    price_change         DOUBLE PRECISION,
    active_tvl_change      DOUBLE PRECISION,
    holders_change        DOUBLE PRECISION,

    vol_tvl            DOUBLE PRECISION,
    fee_tvl            DOUBLE PRECISION,
    fee_active_tvl        DOUBLE PRECISION,
    tau_a              DOUBLE PRECISION,

    sigma_gk           DOUBLE PRECISION,
    sigma_fast          DOUBLE PRECISION,
    sigma_slow          DOUBLE PRECISION,
    sigma_d            DOUBLE PRECISION,
    sigma_jump          DOUBLE PRECISION,

    -- Forecast fee rate, bps precision to match pools.base_fee_bps
    -- and pool_snapshots.total_fee_bps.
    f_hat              NUMERIC(20,6),
    phi_org            DOUBLE PRECISION,
    phi_mech           DOUBLE PRECISION,
    phi_time           DOUBLE PRECISION,
    phi_size           DOUBLE PRECISION,
    r_gross            DOUBLE PRECISION,
    r_org              DOUBLE PRECISION,
    y_fee              DOUBLE PRECISION,

    -- Meteora's weighted-percentile blend, reproduced for contrast (
    --). Independently observable from the public API field set, so it
    -- is safe to reproduce;.
    top_score           DOUBLE PRECISION,

    PRIMARY KEY (pool_address, bucket_start)
);

SELECT create_hypertable(
    'indicators_5m', 'bucket_start',
    chunk_time_interval => INTERVAL '7 days'
);

CREATE INDEX idx_indicators_5m_rank ON indicators_5m (bucket_start DESC, r_org DESC);

CREATE TABLE indicators_10m (
    pool_address       TEXT NOT NULL REFERENCES pools (pool_address),
    venue              SMALLINT NOT NULL,
    bucket_start        TIMESTAMPTZ NOT NULL,
    quality            CHAR(1) NOT NULL CHECK (quality IN ('A', 'B')),
    regime             TEXT,
    vol_change          DOUBLE PRECISION,
    fee_change          DOUBLE PRECISION,
    tvl_change          DOUBLE PRECISION,
    price_change         DOUBLE PRECISION,
    active_tvl_change      DOUBLE PRECISION,
    holders_change        DOUBLE PRECISION,
    vol_tvl            DOUBLE PRECISION,
    fee_tvl            DOUBLE PRECISION,
    fee_active_tvl        DOUBLE PRECISION,
    tau_a              DOUBLE PRECISION,
    sigma_gk           DOUBLE PRECISION,
    sigma_fast          DOUBLE PRECISION,
    sigma_slow          DOUBLE PRECISION,
    sigma_d            DOUBLE PRECISION,
    sigma_jump          DOUBLE PRECISION,
    f_hat              NUMERIC(20,6),
    phi_org            DOUBLE PRECISION,
    phi_mech           DOUBLE PRECISION,
    phi_time           DOUBLE PRECISION,
    phi_size           DOUBLE PRECISION,
    r_gross            DOUBLE PRECISION,
    r_org              DOUBLE PRECISION,
    y_fee              DOUBLE PRECISION,
    top_score           DOUBLE PRECISION,
    PRIMARY KEY (pool_address, bucket_start)
);

-- Native base for tier-0 pools on the RPC backend; rolled up
-- for tier 1, which is what makes this timeframe structural rather than
-- decorative.
SELECT create_hypertable(
    'indicators_10m', 'bucket_start',
    chunk_time_interval => INTERVAL '7 days'
);

CREATE INDEX idx_indicators_10m_rank ON indicators_10m (bucket_start DESC, r_org DESC);

CREATE TABLE indicators_1h (
    pool_address       TEXT NOT NULL REFERENCES pools (pool_address),
    venue              SMALLINT NOT NULL,
    bucket_start        TIMESTAMPTZ NOT NULL,
    quality            CHAR(1) NOT NULL CHECK (quality IN ('A', 'B')),
    regime             TEXT,
    vol_change          DOUBLE PRECISION,
    fee_change          DOUBLE PRECISION,
    tvl_change          DOUBLE PRECISION,
    price_change         DOUBLE PRECISION,
    active_tvl_change      DOUBLE PRECISION,
    holders_change        DOUBLE PRECISION,
    vol_tvl            DOUBLE PRECISION,
    fee_tvl            DOUBLE PRECISION,
    fee_active_tvl        DOUBLE PRECISION,
    tau_a              DOUBLE PRECISION,
    sigma_gk           DOUBLE PRECISION,
    sigma_fast          DOUBLE PRECISION,
    sigma_slow          DOUBLE PRECISION,
    sigma_d            DOUBLE PRECISION,
    sigma_jump          DOUBLE PRECISION,
    f_hat              NUMERIC(20,6),
    phi_org            DOUBLE PRECISION,
    phi_mech           DOUBLE PRECISION,
    phi_time           DOUBLE PRECISION,
    phi_size           DOUBLE PRECISION,
    r_gross            DOUBLE PRECISION,
    r_org              DOUBLE PRECISION,
    y_fee              DOUBLE PRECISION,
    top_score           DOUBLE PRECISION,
    PRIMARY KEY (pool_address, bucket_start)
);

SELECT create_hypertable(
    'indicators_1h', 'bucket_start',
    chunk_time_interval => INTERVAL '7 days'
);

CREATE INDEX idx_indicators_1h_rank ON indicators_1h (bucket_start DESC, r_org DESC);

CREATE TABLE indicators_4h (
    pool_address       TEXT NOT NULL REFERENCES pools (pool_address),
    venue              SMALLINT NOT NULL,
    bucket_start        TIMESTAMPTZ NOT NULL,
    quality            CHAR(1) NOT NULL CHECK (quality IN ('A', 'B')),
    regime             TEXT,
    vol_change          DOUBLE PRECISION,
    fee_change          DOUBLE PRECISION,
    tvl_change          DOUBLE PRECISION,
    price_change         DOUBLE PRECISION,
    active_tvl_change      DOUBLE PRECISION,
    holders_change        DOUBLE PRECISION,
    vol_tvl            DOUBLE PRECISION,
    fee_tvl            DOUBLE PRECISION,
    fee_active_tvl        DOUBLE PRECISION,
    tau_a              DOUBLE PRECISION,
    sigma_gk           DOUBLE PRECISION,
    sigma_fast          DOUBLE PRECISION,
    sigma_slow          DOUBLE PRECISION,
    sigma_d            DOUBLE PRECISION,
    sigma_jump          DOUBLE PRECISION,
    f_hat              NUMERIC(20,6),
    phi_org            DOUBLE PRECISION,
    phi_mech           DOUBLE PRECISION,
    phi_time           DOUBLE PRECISION,
    phi_size           DOUBLE PRECISION,
    r_gross            DOUBLE PRECISION,
    r_org              DOUBLE PRECISION,
    y_fee              DOUBLE PRECISION,
    top_score           DOUBLE PRECISION,
    PRIMARY KEY (pool_address, bucket_start)
);

SELECT create_hypertable(
    'indicators_4h', 'bucket_start',
    chunk_time_interval => INTERVAL '7 days'
);

CREATE INDEX idx_indicators_4h_rank ON indicators_4h (bucket_start DESC, r_org DESC);

CREATE TABLE indicators_24h (
    pool_address       TEXT NOT NULL REFERENCES pools (pool_address),
    venue              SMALLINT NOT NULL,
    bucket_start        TIMESTAMPTZ NOT NULL,
    quality            CHAR(1) NOT NULL CHECK (quality IN ('A', 'B')),
    regime             TEXT,
    vol_change          DOUBLE PRECISION,
    fee_change          DOUBLE PRECISION,
    tvl_change          DOUBLE PRECISION,
    price_change         DOUBLE PRECISION,
    active_tvl_change      DOUBLE PRECISION,
    holders_change        DOUBLE PRECISION,
    vol_tvl            DOUBLE PRECISION,
    fee_tvl            DOUBLE PRECISION,
    fee_active_tvl        DOUBLE PRECISION,
    tau_a              DOUBLE PRECISION,
    sigma_gk           DOUBLE PRECISION,
    sigma_fast          DOUBLE PRECISION,
    sigma_slow          DOUBLE PRECISION,
    sigma_d            DOUBLE PRECISION,
    sigma_jump          DOUBLE PRECISION,
    f_hat              NUMERIC(20,6),
    phi_org            DOUBLE PRECISION,
    phi_mech           DOUBLE PRECISION,
    phi_time           DOUBLE PRECISION,
    phi_size           DOUBLE PRECISION,
    r_gross            DOUBLE PRECISION,
    r_org              DOUBLE PRECISION,
    y_fee              DOUBLE PRECISION,
    top_score           DOUBLE PRECISION,
    PRIMARY KEY (pool_address, bucket_start)
);

SELECT create_hypertable(
    'indicators_24h', 'bucket_start',
    chunk_time_interval => INTERVAL '7 days'
);

CREATE INDEX idx_indicators_24h_rank ON indicators_24h (bucket_start DESC, r_org DESC);
