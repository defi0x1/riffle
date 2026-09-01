-- Real on-chain counterpart to paper_positions (0018): a DLMM PositionV2 account with an
-- owner, a pool and a bin range. Deliberately mirrors paper_positions' shape wherever the same
-- fact is being recorded -- pool_address / venue / opened_at / lower_bin / upper_bin /
-- entry_active_bin / closed_at / close_reason all mean exactly what they mean there, open is
-- `closed_at IS NULL` in both, and nothing here duplicates that as a separate status column.
-- What does not carry over is paper-strategy bookkeeping (signal_id, regime, shape, predicted)
-- and the size columns: a real position's liquidity moves over its life (open, then adds,
-- removes, a close) and each of those is a priced, dated event, so it lives in the
-- position_cash_flows ledger (0031) rather than as a snapshot column that only ever describes
-- entry.
--
-- Per-bin liquidity -- the other half of "a bin range and per-bin liquidity" -- is likewise not
-- a column here. It is reconstructible at any point in time from position_cash_flows'
-- `bin_liquidity` (each deposit/withdrawal's per-bin delta, when the decoder can supply it)
-- summed against bin_states (0008) for the pool, the same way a real position's current value
-- is computed rather than cached. Storing a mutable per-bin table that must be kept in lockstep
-- with on-chain state on every mark is a second source of truth for a number that is already
-- cheap to recompute from the ledger and current chain state.
--
-- Low, per-user-action volume and mutable over its lifecycle (closed_at / close_reason set
-- later), so a plain table like paper_positions, not a hypertable.
CREATE TABLE positions (
    id                  UUID PRIMARY KEY,
    -- The on-chain PositionV2 account pubkey. Unique and known only once the `open` intent
    -- confirms (see confirm_transaction_intent, 0030) -- this is the row's true on-chain
    -- identity, `id` is only this schema's handle for it.
    position_address       TEXT NOT NULL UNIQUE,
    wallet_address         TEXT NOT NULL REFERENCES wallets (pubkey),
    pool_address           TEXT NOT NULL REFERENCES pools (pool_address),
    venue                SMALLINT NOT NULL,
    opened_at             TIMESTAMPTZ NOT NULL,
    entry_active_bin         INTEGER,
    lower_bin             INTEGER NOT NULL,
    upper_bin             INTEGER NOT NULL,
    closed_at             TIMESTAMPTZ,
    close_reason           TEXT
);

-- "What am I holding right now" -- the Mini App's home screen -- for one wallet.
CREATE INDEX idx_positions_wallet_open ON positions (wallet_address) WHERE closed_at IS NULL;
CREATE INDEX idx_positions_pool_opened ON positions (pool_address, opened_at DESC);
