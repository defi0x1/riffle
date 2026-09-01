-- Finalised at t+24h and t+72h (t+14d for the S regime). The evidence base
-- this version exists to build -- retained indefinitely, never dropped
-- (plans/03 §6). Low, episodic volume (a handful of rows per closed
-- position), so a plain table rather than a hypertable.
CREATE TABLE outcomes (
    position_id           UUID NOT NULL REFERENCES paper_positions (id),
    horizon               TEXT NOT NULL,
    venue                 SMALLINT NOT NULL,
    finalized_at            TIMESTAMPTZ NOT NULL,
    fees_real              NUMERIC(38,18),
    fees_predicted           NUMERIC(38,18),
    lvr_real               NUMERIC(38,18),
    -- Dimensionless, same scale as indicators_{tf}.r_org.
    r_real                DOUBLE PRECISION,
    r_predicted             DOUBLE PRECISION,
    -- Fraction of the horizon spent in range, in [0,1].
    time_in_range            DOUBLE PRECISION,
    hit                  BOOLEAN,
    PRIMARY KEY (position_id, horizon)
);

CREATE INDEX idx_outcomes_finalized_at ON outcomes (finalized_at DESC);
