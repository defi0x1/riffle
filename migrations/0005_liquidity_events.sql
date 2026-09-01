CREATE TABLE liquidity_events (
    ts                  TIMESTAMPTZ NOT NULL,
    slot                BIGINT NOT NULL,
    signature            TEXT NOT NULL,
    ix_index             INTEGER NOT NULL,
    pool_address         TEXT NOT NULL REFERENCES pools (pool_address),
    position_address      TEXT,
    owner                TEXT NOT NULL,
    -- 0 = add, 1 = remove. Closed set, unlikely to grow, so a CHECK is safe
    -- here (unlike venue-scoped columns elsewhere).
    action               SMALLINT NOT NULL CHECK (action IN (0, 1)),
    active_bin_id         INTEGER NOT NULL,
    amount_x_raw           NUMERIC(40,0),
    amount_y_raw           NUMERIC(40,0),
    amount_usd            NUMERIC(38,18),
    PRIMARY KEY (pool_address, ts, signature, ix_index)
);

SELECT create_hypertable(
    'liquidity_events', 'ts',
    chunk_time_interval => INTERVAL '1 day'
);

CREATE INDEX idx_liquidity_events_pool_ts ON liquidity_events (pool_address, ts DESC);
