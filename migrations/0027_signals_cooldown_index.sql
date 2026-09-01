-- The signals worker's cooldown now reads the most recent row for a given
-- (pool_address, timeframe, kind) directly from this table instead of an in-memory map, so
-- the cooldown clock survives a restart the same way regime_state/volatility_state (0023)
-- already do. idx_signals_pool_ts leads with pool_address only, so that lookup would still
-- scan every INFO row (written once per pool/timeframe/tick, far outnumbering the
-- POTENTIAL/DEGRADING/GATE_FAIL rows a cooldown check actually cares about) before finding a
-- kind match; this index puts the whole cooldown key first.
CREATE INDEX idx_signals_pool_tf_kind_ts ON signals (pool_address, timeframe, kind, ts DESC);
