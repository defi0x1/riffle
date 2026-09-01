-- Not decoded by the reference indexer we diverge from at all. We need it
-- because a fee-parameter change is an instant exit re-evaluation with no
-- persistence window, and a jack above 2x is a kill condition.
CREATE TABLE fee_param_updates (
    ts                  TIMESTAMPTZ NOT NULL,
    slot                BIGINT NOT NULL,
    signature            TEXT NOT NULL,
    pool_address         TEXT NOT NULL REFERENCES pools (pool_address),
    field                TEXT NOT NULL,
    old_value             BIGINT,
    new_value             BIGINT,
    -- A single transaction can change several fields at once; the natural
    -- key needs `field` to stay unique per event.
    PRIMARY KEY (pool_address, ts, signature, field)
);

-- Rare events (creator fee-parameter changes only) -- a week-wide chunk
-- keeps chunk count low without ever producing an oversized chunk.
SELECT create_hypertable(
    'fee_param_updates', 'ts',
    chunk_time_interval => INTERVAL '7 days'
);

CREATE INDEX idx_fee_param_updates_pool_ts ON fee_param_updates (pool_address, ts DESC);
