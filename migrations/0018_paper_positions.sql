-- Opened when a POTENTIAL signal fires. Never touches chain (:
-- no keys exist anywhere in the process tree). Mutable over its lifecycle
-- (closed_at / close_reason are set later), so a plain table, not a
-- hypertable.

-- `lower_bin` / `upper_bin` are DLMM-specific (a range is expressed in
-- bins); says this table needs only a `venue` column added for
-- venue extension, not a satellite split like pools/pool_snapshots. A
-- future venue's positions simply leave these NULL, the same way `pools`
-- would carry NULL DLMM columns if it were not split -- except here the
-- table is small and mutable rather than the high-volume table the split
-- in 0002/0009 exists to protect, so the plan does not ask for one.
CREATE TABLE paper_positions (
    id                  UUID PRIMARY KEY,
    signal_id            UUID REFERENCES signals (id),
    pool_address          TEXT NOT NULL REFERENCES pools (pool_address),
    venue                SMALLINT NOT NULL,
    opened_at            TIMESTAMPTZ NOT NULL,
    regime               TEXT,
    entry_price           DOUBLE PRECISION,
    entry_active_bin        INTEGER,
    lower_bin             INTEGER,
    upper_bin             INTEGER,
    shape                TEXT,
    size_usd             NUMERIC(38,18),
    size_per_bin           NUMERIC(38,18),
    predicted             JSONB,
    closed_at             TIMESTAMPTZ,
    close_reason           TEXT
);

CREATE INDEX idx_paper_positions_pool_opened ON paper_positions (pool_address, opened_at DESC);
-- Tier demotion exempts a pool with an open paper position;
-- the engine needs this lookup on every demotion sweep.
CREATE INDEX idx_paper_positions_open ON paper_positions (pool_address) WHERE closed_at IS NULL;
