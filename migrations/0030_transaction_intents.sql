-- The full lifecycle of one user action -- open, add, remove, claim or close -- from an unsigned
-- transaction the backend built, through on-device signing, submission, and confirmation or
-- failure. CREATED -> SUBMITTED -> {CONFIRMED | FAILED}, or -> EXPIRED if the client never comes
-- back with a signature; see types::intent_status. Rows are created before a signature exists
-- and mutate in place as the flow progresses, so this is a plain table like paper_positions and
-- transaction_intents, not a hypertable.
--
-- Keyless by construction: `unsigned_tx_base64` is a Solana transaction message with its
-- signature slot still empty -- by definition of the wire format it cannot contain a private
-- key -- and `signature` is filled in only once the client has produced it device-side and
-- submitted it back. No column here, or anywhere in this schema, can hold a private key, seed
-- phrase, keystore blob or passphrase; if a future change seems to need one, that is a sign the
-- design is wrong, not that this table needs a new column.
--
-- Double submission is made structurally impossible, not just unlikely, by two independent
-- constraints:
--   1. UNIQUE (wallet_address, idempotency_key). The Mini App generates one idempotency key per
--      logical action (one button press), before a signature can possibly exist. If the "build
--      me a transaction" request is retried -- a network blip, the app relaunching mid-flow --
--      write::create_transaction_intent resolves to the same row instead of minting a second
--      intent for the same tap, via an INSERT ... ON CONFLICT DO UPDATE ... RETURNING that is a
--      no-op on every field but still returns the existing row.
--   2. UNIQUE (signature) once set. Postgres refuses a second row -- or a second UPDATE -- from
--      ever attaching an already-used signature to a different intent. Even if two intents were
--      somehow created for the same action (constraint 1 bypassed by a bug, say), only one of
--      them can ever be recorded as the confirmation of a real on-chain transaction; the second
--      write fails outright rather than silently double-booking a cash flow.
CREATE TABLE transaction_intents (
    id                    UUID PRIMARY KEY,
    wallet_address             TEXT NOT NULL REFERENCES wallets (pubkey),
    -- NULL only for `open`: the position does not exist until this intent confirms and its
    -- on-chain position account becomes known (see positions.position_address, 0029). Every
    -- other action targets a position that already exists.
    position_id              UUID REFERENCES positions (id),
    pool_address              TEXT NOT NULL REFERENCES pools (pool_address),
    venue                   SMALLINT NOT NULL,
    -- 0 open, 1 add, 2 remove, 3 claim, 4 close. See types::intent_action. Closed set describing
    -- what kind of Solana instruction this is, unlikely to grow the way a venue or signal kind
    -- would -- CHECK is the same call liquidity_events.action already made (0005).
    action                  SMALLINT NOT NULL CHECK (action IN (0, 1, 2, 3, 4)),
    idempotency_key             TEXT NOT NULL,
    -- 0 created, 1 submitted, 2 confirmed, 3 failed, 4 expired. See types::intent_status.
    status                  SMALLINT NOT NULL DEFAULT 0 CHECK (status IN (0, 1, 2, 3, 4)),
    unsigned_tx_base64           TEXT NOT NULL,
    -- Action-specific inputs used to build the transaction (requested bin range, amounts,
    -- slippage tolerance, ...). Kept so an interrupted flow can be resumed without asking the
    -- user to re-enter what they asked for, and so a later audit can see what was requested
    -- versus what actually confirmed.
    params                  JSONB,
    created_at                TIMESTAMPTZ NOT NULL,
    -- Solana blockhashes expire in roughly a minute; a durable-nonce transaction lives longer.
    -- Either way, once past this the intent should be swept to EXPIRED rather than left
    -- CREATED/SUBMITTED forever.
    expires_at                TIMESTAMPTZ,
    signature                TEXT,
    submitted_at               TIMESTAMPTZ,
    confirmed_at               TIMESTAMPTZ,
    slot                    BIGINT,
    failed_at                TIMESTAMPTZ,
    failure_reason              TEXT,
    updated_at                TIMESTAMPTZ NOT NULL
);

CREATE UNIQUE INDEX idx_transaction_intents_wallet_idem ON transaction_intents (wallet_address, idempotency_key);
CREATE UNIQUE INDEX idx_transaction_intents_signature ON transaction_intents (signature) WHERE signature IS NOT NULL;
-- Resuming an interrupted flow only ever looks at a wallet's not-yet-settled intents.
CREATE INDEX idx_transaction_intents_wallet_pending ON transaction_intents (wallet_address, created_at DESC)
    WHERE status IN (0, 1);
