-- V2 registers a Telegram user's own Solana wallets so real positions, balances and profit
-- can be attributed to someone. There is no `telegram_users` table: a Telegram user id is
-- already a stable external identifier (same BIGINT convention as muted_pools.chat_id), and
-- nothing about the user beyond that id is ever stored here.
--
-- Public keys only, by construction: every column below is either public on-chain data or a
-- label the user typed. Nothing in this table -- or anywhere in the V2 schema -- can hold a
-- private key, seed phrase, encrypted keystore blob or passphrase. Signing happens on the
-- user's own device inside the Telegram Mini App; the backend never sees, generates or stores
-- key material, so there is no column for it to leak.
--
-- A pubkey is owned by exactly one Telegram user, forever, enforced by PRIMARY KEY (pubkey):
-- a second INSERT for an already-registered pubkey cannot succeed. Ownership is never
-- reassigned, even after revocation -- write::register_wallet returns an explicit
-- "owned by another user" outcome instead of silently attaching a wallet (and its position,
-- balance and profit history) to a different account. The backend has no way to verify who
-- actually controls a keypair at registration time, so the only safe rule is
-- first-registration-wins; a user who legitimately controls an already-registered wallet must
-- get it revoked under its original owner before it can be registered elsewhere.
CREATE TABLE wallets (
    pubkey              TEXT PRIMARY KEY,
    telegram_user_id     BIGINT NOT NULL,
    label                TEXT,
    registered_at         TIMESTAMPTZ NOT NULL,
    -- Soft unregister only: positions, transaction_intents and wallet_balances all FK to this
    -- row and must keep resolving for historical and audit queries after a user removes a
    -- wallet from their device. Re-registering the same wallet by its same owner clears this.
    revoked_at            TIMESTAMPTZ
);

-- "Which wallets does this user have active" is the hot path (every Mini App screen open);
-- revoked wallets are excluded so it does not grow with abandoned registrations.
CREATE INDEX idx_wallets_telegram_user ON wallets (telegram_user_id) WHERE revoked_at IS NULL;
