-- Read by /status and exported to Prometheus (plans/03 §5, plans/08 §7).
-- Geyser is forward-only, so a slot gap is permanent unless backfilled --
-- it must be visible here, not buried in logs.
CREATE TABLE ingest_health (
    ts                  TIMESTAMPTZ NOT NULL,
    source               TEXT NOT NULL,
    last_slot             BIGINT,
    slot_gap              BIGINT,
    messages              TEXT,
    decode_errors           INTEGER,
    write_latency_ms         INTEGER
);

-- A continuous operational stream with no natural end -- a hypertable, but
-- Prometheus is the durable store for this data (plans/08 §7); Postgres
-- only needs enough history to answer /status, so retention is short
-- relative to the raw layer (0022).
SELECT create_hypertable(
    'ingest_health', 'ts',
    chunk_time_interval => INTERVAL '1 day'
);

CREATE INDEX idx_ingest_health_source_ts ON ingest_health (source, ts DESC);
