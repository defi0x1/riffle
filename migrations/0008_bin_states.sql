-- Twin of 0007's active_bin_snapshots (plans/11 §2, correcting the single
-- `bin_states` table in the original data model). Full ~210-bin
-- distribution, written at a 5-minute cadence, for the two consumers that
-- need many bins but not high frequency:
--   1. fee accrual: fee_*_per_token_stored is monotonically non-decreasing,
--      so accrual between t0 and t1 is a difference of two endpoints --
--      intermediate samples between two 5-minute polls carry no
--      information, so 5-minute endpoints answer exactly what 15-second
--      ones would.
--   2. bin-map rendering / shape analysis.
--
-- Application-level change detection (skip a bin whose
-- (amount_x, amount_y, liquidity_supply, fee_x_per_token_stored,
-- fee_y_per_token_stored) is unchanged since the last write) reduces this
-- further; most bins are idle most of the time.
CREATE TABLE bin_states (
    ts                          TIMESTAMPTZ NOT NULL,
    slot                        BIGINT NOT NULL,
    pool_address                 TEXT NOT NULL REFERENCES pools (pool_address),
    bin_id                       INTEGER NOT NULL,
    amount_x                     NUMERIC(40,0) NOT NULL,
    amount_y                     NUMERIC(40,0) NOT NULL,
    liquidity_supply               NUMERIC(40,0) NOT NULL,
    price_q64                    NUMERIC(40,0) NOT NULL,
    ui_price                     DOUBLE PRECISION NOT NULL,
    fee_x_per_token_stored          NUMERIC(40,0) NOT NULL,
    fee_y_per_token_stored          NUMERIC(40,0) NOT NULL,
    PRIMARY KEY (pool_address, bin_id, ts)
);

-- 100 pools x ~210 bins x 288 samples/day (5-min cadence) is ~6M rows/day
-- before change detection, ~84M over the 14-day retention window (0022).
-- Six-hour chunks keep each chunk to a few hundred thousand rows rather
-- than one multi-million-row chunk per day.
SELECT create_hypertable(
    'bin_states', 'ts',
    chunk_time_interval => INTERVAL '6 hours'
);
