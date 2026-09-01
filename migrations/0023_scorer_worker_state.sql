-- Hysteresis and EWMA state the decision pipeline carries between ticks. Both round-trip
-- through here so a restart does not reset the regime classifier's persistence/cooldown
-- clock or the volatility EWMAs to zero -- recomputing them from scratch on every restart
-- would make the 30-minute persistence window and 2-hour cooldown meaningless. One row per
-- (pool, venue, timeframe): each timeframe's pipeline evaluation is independent, so each
-- carries its own state.

CREATE TABLE regime_state (
    pool_address     TEXT NOT NULL REFERENCES pools (pool_address),
    venue            SMALLINT NOT NULL,
    timeframe        TEXT NOT NULL,
    regime           TEXT,
    since            TIMESTAMPTZ NOT NULL,
    pending          TEXT,
    pending_since    TIMESTAMPTZ,
    last_transition  TIMESTAMPTZ,
    updated_at       TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (pool_address, venue, timeframe)
);

CREATE TABLE volatility_state (
    pool_address         TEXT NOT NULL REFERENCES pools (pool_address),
    venue                SMALLINT NOT NULL,
    timeframe            TEXT NOT NULL,
    sigma_fast_variance  DOUBLE PRECISION NOT NULL,
    sigma_slow_variance  DOUBLE PRECISION NOT NULL,
    first_observed_at    TIMESTAMPTZ NOT NULL,
    updated_at           TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (pool_address, venue, timeframe)
);
