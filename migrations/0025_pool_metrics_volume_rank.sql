-- /volume ranks the latest bucket of pool_metrics_{tf} by raw volume_usd. The existing
-- (bucket_start DESC, pool_address) index on each of these tables serves the "find the
-- latest bucket" half of that query but not the ordering, so this adds the same kind of
-- rank index indicators_{tf} already carries for r_org (0015).
CREATE INDEX idx_pool_metrics_5m_volume ON pool_metrics_5m (bucket_start DESC, volume_usd DESC);
CREATE INDEX idx_pool_metrics_10m_volume ON pool_metrics_10m (bucket_start DESC, volume_usd DESC);
CREATE INDEX idx_pool_metrics_1h_volume ON pool_metrics_1h (bucket_start DESC, volume_usd DESC);
CREATE INDEX idx_pool_metrics_4h_volume ON pool_metrics_4h (bucket_start DESC, volume_usd DESC);
CREATE INDEX idx_pool_metrics_24h_volume ON pool_metrics_24h (bucket_start DESC, volume_usd DESC);
