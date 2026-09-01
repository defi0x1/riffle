-- Raw swap events, whole-program flow on the Geyser backend. The largest
-- raw table by a wide margin; see 0022 for why its retention starts short.
CREATE TABLE swaps (
    ts                  TIMESTAMPTZ NOT NULL,
    slot                BIGINT NOT NULL,
    signature            TEXT NOT NULL,
    ix_index             INTEGER NOT NULL,
    pool_address         TEXT NOT NULL REFERENCES pools (pool_address),
    -- Keyed on for the wash-trade screen and the timing classifier
    --.
    signer               TEXT NOT NULL,
    -- Convention fixed here because it is easy to get backwards:
    -- swap_for_y = true means selling X for Y, i.e. sell-side volume.
    swap_for_y           BOOLEAN NOT NULL,
    amount_in_raw         NUMERIC(40,0) NOT NULL,
    amount_out_raw         NUMERIC(40,0) NOT NULL,
    amount_in            NUMERIC(38,18) NOT NULL,
    amount_out            NUMERIC(38,18) NOT NULL,
    start_bin_id          INTEGER NOT NULL,
    end_bin_id            INTEGER NOT NULL,
    -- Derived from start_bin_id / end_bin_id; together these form the OHLC
    -- price series pool_metrics_5m is built from.
    start_price           NUMERIC(38,18),
    end_price             NUMERIC(38,18),
    -- Includes the protocol cut. LP share is fee_raw - protocol_fee_raw.
    fee_raw               NUMERIC(40,0) NOT NULL,
    protocol_fee_raw       NUMERIC(40,0) NOT NULL,
    host_fee_raw           NUMERIC(40,0),
    fee_bps               NUMERIC(20,6) NOT NULL,
    volume_usd            NUMERIC(38,18),
    trade_fee_usd          NUMERIC(38,18),
    protocol_fee_usd        NUMERIC(38,18),
    PRIMARY KEY (pool_address, ts, signature, ix_index)
);

-- Idempotent restart: reprocessing a slot range after a restart is a no-op
-- because the PK includes (signature, ix_index).
SELECT create_hypertable(
    'swaps', 'ts',
    chunk_time_interval => INTERVAL '1 day'
);

CREATE INDEX idx_swaps_pool_ts ON swaps (pool_address, ts DESC);
