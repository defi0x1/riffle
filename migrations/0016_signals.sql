-- Part of the derived layer's evidence base -- retained indefinitely, never
-- dropped. A plain table, not a hypertable: TimescaleDB
-- requires any unique index on a hypertable to include the partitioning
-- column, and a bare UUID primary key referenced by rationale (0017) is
-- worth more here than compression on a table whose volume is episodic
-- (one row per evaluation that actually produces a signal, not per tick).
CREATE TABLE signals (
    id                  UUID PRIMARY KEY,
    ts                  TIMESTAMPTZ NOT NULL,
    pool_address         TEXT NOT NULL REFERENCES pools (pool_address),
    venue                SMALLINT NOT NULL,
    timeframe             TEXT NOT NULL,
    -- POTENTIAL | DEGRADING | GATE_FAIL | INFO. Left as TEXT, not a CHECK
    -- enum: new kinds are an application-layer addition, not a migration.
    kind                 TEXT NOT NULL,
    regime               TEXT,
    numbers              JSONB,
    -- Stamped on every signal so a bad config cannot ship quietly and
    -- /status can show what produced it.
    config_hash            TEXT NOT NULL,
    expires_at            TIMESTAMPTZ
);

CREATE INDEX idx_signals_pool_ts ON signals (pool_address, ts DESC);
CREATE INDEX idx_signals_kind_ts ON signals (kind, ts DESC);
CREATE INDEX idx_signals_expires_at ON signals (expires_at);
