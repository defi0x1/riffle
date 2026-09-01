-- Real-position analogue of position_marks (0019): marked on the same kind of cadence, for the
-- same reason -- "what is this worth right now" needs a standing time series, not just a value
-- computed on demand, because the counterfactual profit needs (item 3: what the deposited
-- tokens would be worth if simply held) has to be reproducible from a specific point in time,
-- not only from today's price. Genuinely append-only and is itself evidence a profit number is
-- recomputed from, so it is a hypertable with the same "keep forever, compress once cold"
-- treatment 0022 gives position_marks, not the 90-day raw-event default.
--
-- This is a new table rather than a widened position_marks. position_marks' primary key and FK
-- both target paper_positions specifically, and Postgres has no polymorphic foreign key that
-- would let one time series legitimately reference either paper_positions or positions
-- depending on the row -- the shared-parent-table trick used for pools/dlmm_pool_params (0002)
-- does not fit here, since paper_positions and positions describe genuinely different lifecycles
-- (paper positions carry predicted/regime bookkeeping a real position never has) rather than one
-- shared shape with venue-specific extensions. Mirroring the column shape into its own table is
-- the honest version of "reuse the marking design": the same marking logic can be pointed at
-- either table with a struct and a table name change, without a fragile polymorphic reference or
-- an ALTER on a hypertable that is already carrying months of paper-trading history.
--
-- `price_x_usd` / `price_y_usd` are NUMERIC, not DOUBLE PRECISION like position_marks.price.
-- This table's numbers get multiplied through into a user's real cost basis and profit
-- (position_cash_flows, 0031); a float round trip has already lost precision once in this
-- schema (paper_positions.entry_price, position_marks.price) and it does not get to happen
-- again where an actual balance is on the line.
CREATE TABLE position_valuations (
    position_id           UUID NOT NULL REFERENCES positions (id),
    ts                  TIMESTAMPTZ NOT NULL,
    price_x_usd            NUMERIC(38,18),
    price_y_usd            NUMERIC(38,18),
    active_bin_id           INTEGER,
    -- Token amounts the position's liquidity is worth right now, decomposed the same way a
    -- withdrawal would return them -- the other half of value_usd, kept so it is not the only
    -- way to answer "how much of each token is in here".
    amount_x              NUMERIC(38,18),
    amount_y              NUMERIC(38,18),
    -- Fees earned but not yet claimed on-chain -- distinct from position_cash_flows' fee_claim
    -- rows, which only exist once a claim has actually confirmed.
    fees_x_uncollected        NUMERIC(38,18),
    fees_y_uncollected        NUMERIC(38,18),
    value_usd             NUMERIC(38,18),
    -- The counterfactual this position is judged against (item 3): the tokens deposited so far
    -- (position_cash_flows, kind = deposit), valued at this mark's own prices instead of the
    -- position's current composition. Stored per mark rather than only computed at read time so
    -- a profit-vs-holding number from months ago stays reproducible even once the live price
    -- feed no longer answers for that timestamp.
    hold_value_usd          NUMERIC(38,18),
    in_range              BOOLEAN,
    PRIMARY KEY (position_id, ts)
);

-- Volume bounded by the number of concurrently open real positions -- smaller, at least at
-- first, than the paper-position count position_marks' own chunk sizing was reasoned from
-- (0019) -- so the same chunk interval is comfortably wide rather than tight.
SELECT create_hypertable(
    'position_valuations', 'ts',
    chunk_time_interval => INTERVAL '7 days'
);
