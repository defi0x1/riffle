-- Retention and compression for the two hypertables V2 adds, collected here as its own concern
-- the same way 0022 collected V1's -- the rule is unchanged: raw data expires, evidence a
-- number is recomputed from is kept forever.

-- position_valuations: the evidence a profit figure is recomputed from (0032), so it gets
-- exactly the treatment 0022 gave position_marks for the same reason -- compressed once cold,
-- never dropped.
ALTER TABLE position_valuations SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'position_id',
    timescaledb.compress_orderby = 'ts DESC'
);
SELECT add_compression_policy('position_valuations', INTERVAL '30 days');

-- wallet_balances: a raw refresh snapshot, not evidence -- the same category active_bin_snapshots
-- and pool_snapshots are in (0022), so it takes their retention window rather than being kept
-- indefinitely.
ALTER TABLE wallet_balances SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'wallet_address',
    timescaledb.compress_orderby = 'ts DESC'
);
SELECT add_compression_policy('wallet_balances', INTERVAL '7 days');
SELECT add_retention_policy('wallet_balances', INTERVAL '90 days');
