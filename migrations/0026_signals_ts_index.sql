-- /status surfaces the config_hash of the newest signal across every pool and kind, as the
-- best available proxy for "what configuration is currently applied" (see the config_hash
-- comment on 0016). idx_signals_pool_ts and idx_signals_kind_ts both lead with a different
-- column, so neither serves a plain "most recent row" scan.
CREATE INDEX idx_signals_ts ON signals (ts DESC);
