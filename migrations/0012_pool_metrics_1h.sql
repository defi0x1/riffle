-- First tier of the real hierarchical continuous aggregates. Built from
-- pool_metrics_10m (0011), not pool_metrics_5m: every pool has a 10-minute
-- base (native for tier 0, rolled up for tier 1), so building from 10m
-- means this and everything above it is uniform across tiers -- "rollups
-- build from whichever base a pool has" (plans/11 §3) collapses into one
-- rule once 10m has already reconciled the two bases.
--
-- Sums add; OHLC uses first/max/min/last ordered by bucket_start.
CREATE MATERIALIZED VIEW pool_metrics_1h
WITH (timescaledb.continuous) AS
SELECT
    pool_address,
    time_bucket('1 hour', bucket_start) AS bucket_start,

    sum(volume_usd)          AS volume_usd,
    sum(buy_volume_usd)       AS buy_volume_usd,
    sum(sell_volume_usd)      AS sell_volume_usd,
    sum(trade_fee_usd)        AS trade_fee_usd,
    sum(protocol_fee_usd)      AS protocol_fee_usd,
    sum(swap_count)          AS swap_count,
    -- Not a true distinct-trader count across the wider bucket (that would
    -- need the raw signer set); an upper bound via sum-of-buckets. Exact
    -- unique_traders per hour is computed directly from swaps by the
    -- application where it is load-bearing (the wash screen).
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
FROM pool_metrics_10m
GROUP BY pool_address, time_bucket('1 hour', bucket_start)
WITH NO DATA;

SELECT add_continuous_aggregate_policy('pool_metrics_1h',
    start_offset => INTERVAL '3 hours',
    end_offset => INTERVAL '10 minutes',
    schedule_interval => INTERVAL '15 minutes'
);

CREATE INDEX idx_pool_metrics_1h_bucket ON pool_metrics_1h (bucket_start DESC, pool_address);
