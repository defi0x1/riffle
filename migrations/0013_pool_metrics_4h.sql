-- Hierarchical: built from pool_metrics_1h, not raw. TimescaleDB supports
-- continuous aggregates over continuous aggregates as long as the bucket
-- width is a multiple of the source's, so each higher timeframe is
-- incremental rather than rescanning 10m data.
CREATE MATERIALIZED VIEW pool_metrics_4h
WITH (timescaledb.continuous) AS
SELECT
    pool_address,
    time_bucket('4 hours', bucket_start) AS bucket_start,

    sum(volume_usd)          AS volume_usd,
    sum(buy_volume_usd)       AS buy_volume_usd,
    sum(sell_volume_usd)      AS sell_volume_usd,
    sum(trade_fee_usd)        AS trade_fee_usd,
    sum(protocol_fee_usd)      AS protocol_fee_usd,
    sum(swap_count)          AS swap_count,
    sum(unique_traders)        AS unique_traders,

    first(price_open, bucket_start)  AS price_open,
    max(price_high)          AS price_high,
    min(price_low)           AS price_low,
    last(price_close, bucket_start)  AS price_close,
    last(tvl_close, bucket_start)   AS tvl_close,
    last(active_tvl_close, bucket_start) AS active_tvl_close,
    last(active_tvl_median, bucket_start) AS active_tvl_median,
    first(active_bin_open, bucket_start) AS active_bin_open,
    last(active_bin_close, bucket_start) AS active_bin_close,
    last(va_close, bucket_start)    AS va_close,
    last(total_fee_bps_close, bucket_start) AS total_fee_bps_close,
    last(reserve_x_close, bucket_start) AS reserve_x_close,
    last(reserve_y_close, bucket_start) AS reserve_y_close,

    sum(net_deposit_usd)       AS net_deposit_usd,
    sum(add_count)           AS add_count,
    sum(remove_count)         AS remove_count,
    sum(lp_count_delta)        AS lp_count_delta
FROM pool_metrics_1h
GROUP BY pool_address, time_bucket('4 hours', bucket_start)
WITH NO DATA;

SELECT add_continuous_aggregate_policy('pool_metrics_4h',
    start_offset => INTERVAL '12 hours',
    end_offset => INTERVAL '1 hour',
    schedule_interval => INTERVAL '1 hour'
);

CREATE INDEX idx_pool_metrics_4h_bucket ON pool_metrics_4h (bucket_start DESC, pool_address);
