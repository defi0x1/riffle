-- Same shared/satellite split as 0002, applied to state.
-- pool_snapshots holds what is true of any AMM's state; dlmm_pool_state
-- holds what only DLMM has (active bin, the volatility accumulator triple).
-- A DAMM v2 satellite (sqrt price bounds, its own accumulator shape) is a
-- later CREATE TABLE against the same pool_snapshots rows.

-- Pool state sampled on every on-chain account update for the pool, forced
-- at each 5-minute boundary so pool_metrics_5m always has a state row to
-- join flow against even on a quiet pool.
CREATE TABLE pool_snapshots (
    ts                  TIMESTAMPTZ NOT NULL,
    slot                BIGINT NOT NULL,
    pool_address         TEXT NOT NULL REFERENCES pools (pool_address),
    price                DOUBLE PRECISION NOT NULL,
    reserve_x_raw          NUMERIC(40,0),
    reserve_y_raw          NUMERIC(40,0),
    -- Our own reimplementation of the lopsided-pool defensive rule
    -- never a naive x_usd + y_usd. The threshold is
    -- chosen and justified in the application layer, not in this schema.
    tvl_usd              NUMERIC(38,18),
    -- L_a: active-bin TVL, from bin_states at this slot.
    active_tvl_usd         NUMERIC(38,18),
    -- Live on-chain accumulator value; the *current* fee, as opposed to
    -- indicators_{tf}.f_hat, the *forecast* fee.
    total_fee_bps          NUMERIC(20,6) NOT NULL,
    PRIMARY KEY (pool_address, ts)
);

SELECT create_hypertable(
    'pool_snapshots', 'ts',
    chunk_time_interval => INTERVAL '1 day'
);

CREATE INDEX idx_pool_snapshots_pool_ts ON pool_snapshots (pool_address, ts DESC);

-- DLMM-only state. No FK back to pool_snapshots: TimescaleDB requires a
-- unique index on a hypertable to include its time-partitioning column, so
-- a clean (pool_address, ts) FK across two independently-chunked
-- hypertables buys nothing a shared write path does not already give us,
-- and would block dropping old chunks under the retention policy.
CREATE TABLE dlmm_pool_state (
    ts                          TIMESTAMPTZ NOT NULL,
    pool_address                 TEXT NOT NULL REFERENCES pools (pool_address),
    active_bin_id                 INTEGER NOT NULL,
    -- va: the on-chain volatility accumulator.
    volatility_accumulator          INTEGER NOT NULL,
    volatility_reference           INTEGER NOT NULL,
    index_reference                INTEGER NOT NULL,
    -- On-chain clock; never wall clock.
    last_update_timestamp           BIGINT NOT NULL,
    -- the static component, and the dynamic component derived from va
    -- at this slot. total_fee_bps on pool_snapshots is min(base+dynamic, 10%).
    base_fee_bps                  NUMERIC(20,6) NOT NULL,
    dynamic_fee_bps                NUMERIC(20,6) NOT NULL,
    PRIMARY KEY (pool_address, ts)
);

SELECT create_hypertable(
    'dlmm_pool_state', 'ts',
    chunk_time_interval => INTERVAL '1 day'
);
