-- Persists /mute across a bot restart. Keyed by (pool_address, chat_id): the same pool can
-- carry an independent mute per chat, since more than one operator chat can watch the same
-- pool. Low, operator-driven volume and mutable (a re-mute just moves `until`), so a plain
-- table like paper_positions rather than a hypertable.
CREATE TABLE muted_pools (
    pool_address    TEXT NOT NULL REFERENCES pools (pool_address),
    chat_id         BIGINT NOT NULL,
    until           TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (pool_address, chat_id)
);

-- /potential checks every ranked pool against one chat's mute set; expiry is the `until >
-- now()` predicate in that query, not a sweeper job, so a stale row just sits here as
-- harmless disk until the next mute or query touches it.
CREATE INDEX idx_muted_pools_chat_until ON muted_pools (chat_id, until);
