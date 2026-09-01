-- Marked every 5 minutes for the lifetime of a paper position. Genuinely
-- append-only time series and part of the evidence base outcomes (0020)
-- are scored against, so it is a hypertable like the raw/state layer
-- rather than a plain table like paper_positions itself.
CREATE TABLE position_marks (
    position_id           UUID NOT NULL REFERENCES paper_positions (id),
    ts                  TIMESTAMPTZ NOT NULL,
    price                DOUBLE PRECISION,
    active_bin_id          INTEGER,
    -- Delta fee_*_per_token_stored x hypothetical shares, summed per bin
    -- the same accrual logic bin_states exists to support.
    fees_accrued_usd        NUMERIC(38,18),
    -- impermanent loss at the current price delta.
    il_usd               NUMERIC(38,18),
    value_usd             NUMERIC(38,18),
    in_range              BOOLEAN,
    PRIMARY KEY (position_id, ts)
);

-- Volume is bounded by the number of concurrently open positions, which is
-- small (tier 1 is capped at ~100 pools); a wide chunk keeps chunk count
-- low without ever approaching an oversized chunk.
SELECT create_hypertable(
    'position_marks', 'ts',
    chunk_time_interval => INTERVAL '7 days'
);
