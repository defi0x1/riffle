-- Split from a single bin-state table, deliberately (see 0008 for the twin
-- and the full reasoning). This table carries the high-frequency signal:
-- one row per pool per poll, tracking only the *active* bin.
--
-- L-bar_a is a 60-minute median of active-bin liquidity, so it needs one
-- bin at high frequency -- not the full ~210-bin distribution. Writing the
-- full distribution at poll cadence was computed (not assumed) to be ~121M
-- rows/day at 100 pools; this table alone is ~576k rows/day, a ~99.5%
-- reduction with no fidelity loss for the one consumer that needs
-- high-frequency data.
CREATE TABLE active_bin_snapshots (
    ts                  TIMESTAMPTZ NOT NULL,
    slot                BIGINT NOT NULL,
    pool_address         TEXT NOT NULL REFERENCES pools (pool_address),
    bin_id               INTEGER NOT NULL,
    amount_x             NUMERIC(40,0) NOT NULL,
    amount_y             NUMERIC(40,0) NOT NULL,
    liquidity_supply       NUMERIC(40,0) NOT NULL,
    -- L_a: the active-bin quote value the median is taken over.
    quote_value_usd        NUMERIC(38,18),
    PRIMARY KEY (pool_address, ts)
);

SELECT create_hypertable(
    'active_bin_snapshots', 'ts',
    chunk_time_interval => INTERVAL '1 day'
);

-- Retention and compression policies for every hypertable are consolidated
-- in 0022, one concern per file.
