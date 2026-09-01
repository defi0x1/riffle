-- The cost-basis ledger: every confirmed transaction that moved value across a position's
-- boundary -- a deposit (open or add), a withdrawal (remove or close), or a fee claim. Profit
-- is never stored as a bare number anywhere in this schema; it is always computed from these
-- rows plus a position_valuations mark (0032), so a profit figure shown to a user can always be
-- re-derived and audited later, not just trusted.
--
-- Denomination: `amount_x` / `amount_y` keep the exact token quantities that moved, and
-- `price_x_usd` / `price_y_usd` are that token's USD price at `ts`, captured here rather than
-- looked up from a price feed later. Together they make a profit number reconstructible in
-- either direction that is cheap to derive from the same rows: USD (multiply through, sum
-- deposits against withdrawals-plus-current-value -- what a user is shown, and the only
-- denomination that is comparable across positions in differently-paired pools) or native
-- token terms (compare quantities directly, ignoring the price columns entirely -- what "am I
-- up in SOL, not just in dollars" needs). USD is the one this schema already uses as its
-- numeraire everywhere else (volume_usd, tvl_usd, amount_usd), so it is the headline unit here
-- too; nothing about token-denominated profit requires a different row shape, only a different
-- read of the same one.
--
-- Keyed on the transaction_intent that produced it, not a surrogate id: a confirmation is
-- processed by resolving the intent (already unique per on-chain signature, 0030) and inserting
-- its cash flow row once. Replaying a confirmation -- the same money-safety concern
-- transaction_intents itself exists for -- can only ever produce that one row, never a second.
--
-- Low, per-user-action volume, append-only but never mutated after insert, so a plain table
-- like paper_positions and outcomes rather than a hypertable.
CREATE TABLE position_cash_flows (
    transaction_intent_id      UUID PRIMARY KEY REFERENCES transaction_intents (id),
    position_id             UUID NOT NULL REFERENCES positions (id),
    -- 0 deposit, 1 withdrawal, 2 fee claim. See types::cash_flow_kind. Closed set, same call as
    -- transaction_intents.action.
    kind                   SMALLINT NOT NULL CHECK (kind IN (0, 1, 2)),
    ts                    TIMESTAMPTZ NOT NULL,
    amount_x_raw              NUMERIC(40,0),
    amount_y_raw              NUMERIC(40,0),
    amount_x                NUMERIC(38,18),
    amount_y                NUMERIC(38,18),
    price_x_usd              NUMERIC(38,18),
    price_y_usd              NUMERIC(38,18),
    -- Denormalised amount_x * price_x_usd + amount_y * price_y_usd, kept alongside the inputs
    -- it was computed from rather than instead of them.
    value_usd               NUMERIC(38,18),
    -- Per-bin liquidity delta from the decoded instruction, when the decoder can supply it.
    -- Open-ended and venue-specific like paper_positions.predicted / signals.numbers, and not
    -- itself queried by SQL predicates -- only ever read back whole by the caller that computes
    -- a position's current per-bin composition from the full ledger.
    bin_liquidity             JSONB
);

CREATE INDEX idx_position_cash_flows_position_ts ON position_cash_flows (position_id, ts);
