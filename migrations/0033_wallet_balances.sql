-- Token balances for a registered wallet, refreshed on a fixed poll cadence -- enough for the
-- Mini App to show a user what they hold before they act. Append-only time series like
-- pool_snapshots / active_bin_snapshots: a fresh row lands on every refresh regardless of
-- whether the balance actually moved, and "what do I hold right now" is just the latest row per
-- (wallet_address, mint), the same shape of query as "latest pool_snapshots row per pool"
-- (see queries::reconciliation's DISTINCT ON pattern).
--
-- No FK from `mint` to tokens(mint), for the same reason pools does not FK token_x/token_y
-- there (0002): a wallet can hold any SPL mint, discovered here before -- or never -- fetched
-- into `tokens`, and forcing insert order would couple a balance refresh to the token-metadata
-- fetch path for no benefit.
CREATE TABLE wallet_balances (
    wallet_address         TEXT NOT NULL REFERENCES wallets (pubkey),
    mint                 TEXT NOT NULL,
    ts                  TIMESTAMPTZ NOT NULL,
    amount_raw            NUMERIC(40,0) NOT NULL,
    amount                NUMERIC(38,18) NOT NULL,
    price_usd             NUMERIC(38,18),
    value_usd             NUMERIC(38,18),
    PRIMARY KEY (wallet_address, mint, ts)
);

SELECT create_hypertable(
    'wallet_balances', 'ts',
    chunk_time_interval => INTERVAL '7 days'
);

CREATE INDEX idx_wallet_balances_wallet_ts ON wallet_balances (wallet_address, ts DESC);
