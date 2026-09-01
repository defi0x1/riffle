-- Mint metadata and the risk-gate cache (plans/04 §3). Slow-changing,
-- upserted on discovery and refreshed on the gate's own cadence.
CREATE TABLE tokens (
    mint                TEXT PRIMARY KEY,
    symbol              TEXT,
    name                TEXT,
    decimals            SMALLINT NOT NULL,
    -- Must be NULL to pass the risk gate.
    mint_authority      TEXT,
    freeze_authority    TEXT,
    -- SPL Token vs Token-2022 program id.
    token_program       TEXT NOT NULL,
    -- Token-2022 extension flags (PermanentDelegate, TransferHook, transfer
    -- fee bps, ...). Kept as JSONB because the extension set is open-ended
    -- and only a handful of fields are ever queried directly; those few are
    -- checked by the gate logic in the application, not by SQL predicates.
    extensions          JSONB,
    supply              NUMERIC(40,0),
    holder_count        INTEGER,
    -- Fractions in [0,1], not basis points -- matches how the gate
    -- thresholds (< 0.35, < 0.15) are written in plans/04 §3.
    top10_share         DOUBLE PRECISION,
    top1_share          DOUBLE PRECISION,
    is_verified         BOOLEAN,
    rugcheck_score      INTEGER,
    rugcheck_flags      JSONB,
    -- Cache timestamp: risk gate reuses this for 10 min (tier 1 / V2 cadence)
    -- or 6 h (S, V1) before refetching, per plans/04 §3.
    rugcheck_at         TIMESTAMPTZ,
    updated_at          TIMESTAMPTZ NOT NULL
);
