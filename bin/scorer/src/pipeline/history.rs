//! Turns a run of `pool_metrics_{tf}` rows into the historical inputs `engine::screen`/`rank`
//! need beyond the current bucket's own reading: the OHLC bar, trailing log returns, 5-minute
//! autocorrelation, the 24h/7d volume and fee/TVL windows, and the previous-bucket deltas.
//! Pure and independent of storage's row types beyond the plain struct it reads, so it is
//! testable with hand-built fixtures.

use engine::PreviousBucket;
use engine::volatility::OhlcBar;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use storage::queries::PoolMetricsHistoryRow;
use storage::types::Timeframe;

pub struct AssembledInputs {
    pub latest_bar: OhlcBar,
    pub autocorrelations: Vec<f64>,
    pub log_returns_24h: Vec<f64>,
    pub vol_24h: f64,
    pub volume_trend: f64,
    pub fee_tvl_1h: Option<f64>,
    pub fee_tvl_24h: Option<f64>,
    pub fee_tvl_7d: Option<f64>,
    pub n_trades: u32,
    pub tvl_usd: f64,
    pub previous: PreviousBucket,
}

fn bars_per_day(tf: Timeframe) -> usize {
    match tf {
        Timeframe::M5 => 288,
        Timeframe::M10 => 144,
        Timeframe::H1 => 24,
        Timeframe::H4 => 6,
        Timeframe::H24 => 1,
    }
}

fn dec_f64(d: Option<Decimal>) -> Option<f64> {
    d.and_then(|v| v.to_f64())
}

/// `history` must be ordered newest-first (as returned by `storage::queries::pool_metrics_recent`).
/// Returns `None` when there is no current bucket to evaluate at all.
pub fn assemble(
    history: &[PoolMetricsHistoryRow],
    timeframe: Timeframe,
) -> Option<AssembledInputs> {
    let current = history.first()?;
    let latest_bar = OhlcBar {
        open: current.price_open?,
        high: current.price_high?,
        low: current.price_low?,
        close: current.price_close?,
    };

    // Chronological order for return/window math.
    let asc: Vec<&PoolMetricsHistoryRow> = history.iter().rev().collect();

    let closes: Vec<f64> = asc.iter().filter_map(|r| r.price_close).collect();
    let log_returns: Vec<f64> = closes
        .windows(2)
        .filter(|w| w[0] > 0.0 && w[1] > 0.0)
        .map(|w| (w[1] / w[0]).ln())
        .collect();

    // The variance-ratio correction is specified against 5-minute bars; other timeframes get
    // the naive (uncorrected) daily variance, which `dlmm_math::daily_vol` already handles
    // gracefully for an empty autocorrelation slice.
    let autocorrelations = if timeframe == Timeframe::M5 {
        lag_autocorrelations(&log_returns, 6)
    } else {
        Vec::new()
    };

    let day_bars = bars_per_day(timeframe).min(asc.len());
    let log_returns_24h = if log_returns.len() >= day_bars {
        log_returns[log_returns.len() - day_bars..].to_vec()
    } else {
        log_returns.clone()
    };

    let vol_24h = window_sum(&asc, day_bars, |r| r.volume_usd).unwrap_or(0.0);

    let fee_tvl_1h = fee_tvl_over(&asc, window_bars(timeframe, 1.0));
    let fee_tvl_24h = fee_tvl_over(&asc, window_bars(timeframe, 24.0));
    let fee_tvl_7d = fee_tvl_over(&asc, window_bars(timeframe, 24.0 * 7.0));

    let volume_trend = volume_trend_wk_wk(&asc, day_bars);

    let n_trades = current.swap_count.map(|n| n.max(0) as u32).unwrap_or(0);
    let tvl_usd = dec_f64(current.tvl_close).unwrap_or(0.0);

    let previous = if history.len() >= 2 {
        let prev = &history[1];
        PreviousBucket {
            vol: delta(current.volume_usd, prev.volume_usd),
            fee: delta(current.trade_fee_usd, prev.trade_fee_usd),
            tvl: delta(current.tvl_close, prev.tvl_close),
            price: current
                .price_close
                .zip(prev.price_close)
                .map(|(c, p)| c - p),
            active_tvl: delta(current.active_tvl_close, prev.active_tvl_close),
            holders: None,
        }
    } else {
        PreviousBucket::default()
    };

    Some(AssembledInputs {
        latest_bar,
        autocorrelations,
        log_returns_24h,
        vol_24h,
        volume_trend,
        fee_tvl_1h,
        fee_tvl_24h,
        fee_tvl_7d,
        n_trades,
        tvl_usd,
        previous,
    })
}

fn delta(current: Option<Decimal>, previous: Option<Decimal>) -> Option<f64> {
    let c = dec_f64(current)?;
    let p = dec_f64(previous)?;
    Some(c - p)
}

fn window_bars(tf: Timeframe, window_hours: f64) -> usize {
    let per_day = bars_per_day(tf) as f64;
    (window_hours / 24.0 * per_day).round() as usize
}

fn window_sum(
    asc: &[&PoolMetricsHistoryRow],
    bars: usize,
    field: impl Fn(&PoolMetricsHistoryRow) -> Option<Decimal>,
) -> Option<f64> {
    if bars == 0 || asc.len() < bars {
        return None;
    }
    let window = &asc[asc.len() - bars..];
    let mut total = Decimal::ZERO;
    let mut any = false;
    for r in window {
        if let Some(v) = field(r) {
            total += v;
            any = true;
        }
    }
    any.then(|| total.to_f64()).flatten()
}

fn fee_tvl_over(asc: &[&PoolMetricsHistoryRow], bars: usize) -> Option<f64> {
    if bars == 0 || asc.len() < bars {
        return None;
    }
    let fee = window_sum(asc, bars, |r| r.trade_fee_usd)?;
    let tvl = dec_f64(asc.last()?.tvl_close)?;
    if tvl <= 0.0 {
        return None;
    }
    Some(fee / tvl)
}

// Not a true 7-day-ago comparison at every timeframe (a pool younger than two windows has no
// prior period to compare against), so this degrades to 0.0 -- neutral, neither a decay nor a
// spike -- rather than fabricating a trend from partial data.
fn volume_trend_wk_wk(asc: &[&PoolMetricsHistoryRow], day_bars: usize) -> f64 {
    if day_bars == 0 || asc.len() < day_bars * 2 {
        return 0.0;
    }
    let recent = window_sum(asc, day_bars, |r| r.volume_usd).unwrap_or(0.0);
    let prior_slice = &asc[asc.len() - day_bars * 2..asc.len() - day_bars];
    let prior: f64 = prior_slice
        .iter()
        .filter_map(|r| dec_f64(r.volume_usd))
        .sum();
    if prior <= 0.0 {
        return 0.0;
    }
    (recent - prior) / prior
}

fn lag_autocorrelations(returns: &[f64], max_lag: usize) -> Vec<f64> {
    if returns.len() < 20 {
        return Vec::new();
    }
    let mean = returns.iter().sum::<f64>() / returns.len() as f64;
    let variance: f64 =
        returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / returns.len() as f64;
    if variance <= 0.0 {
        return Vec::new();
    }

    (1..=max_lag.min(returns.len() - 1))
        .map(|lag| {
            let cov: f64 = returns
                .windows(lag + 1)
                .map(|w| (w[0] - mean) * (w[lag] - mean))
                .sum::<f64>()
                / (returns.len() - lag) as f64;
            cov / variance
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};

    fn t(n: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
            + chrono::Duration::minutes(n * 5)
    }

    fn row(n: i64, close: f64, volume: i64, fee: i64, tvl: i64) -> PoolMetricsHistoryRow {
        PoolMetricsHistoryRow {
            bucket_start: t(n),
            volume_usd: Some(Decimal::new(volume, 0)),
            trade_fee_usd: Some(Decimal::new(fee, 0)),
            swap_count: Some(10),
            unique_traders: Some(3),
            price_open: Some(close),
            price_high: Some(close * 1.01),
            price_low: Some(close * 0.99),
            price_close: Some(close),
            tvl_close: Some(Decimal::new(tvl, 0)),
            active_tvl_close: Some(Decimal::new(tvl / 10, 0)),
            active_tvl_median: Some(Decimal::new(tvl / 10, 0)),
            active_bin_close: Some(100),
            total_fee_bps_close: Some(Decimal::new(30, 2)),
        }
    }

    #[test]
    fn test_empty_history_yields_none() {
        assert!(assemble(&[], Timeframe::M5).is_none());
    }

    #[test]
    fn test_single_row_has_no_previous_delta() {
        let history = vec![row(0, 1.0, 1_000, 3, 100_000)];
        let out = assemble(&history, Timeframe::M5).unwrap();
        assert_eq!(out.previous.vol, None);
        assert_eq!(out.latest_bar.close, 1.0);
    }

    #[test]
    fn test_previous_bucket_is_a_delta_not_a_raw_value() {
        // Newest first, as storage returns it.
        let history = vec![
            row(1, 1.05, 1_500, 5, 110_000),
            row(0, 1.0, 1_000, 3, 100_000),
        ];
        let out = assemble(&history, Timeframe::M5).unwrap();
        assert_eq!(out.previous.vol, Some(500.0));
        assert_eq!(out.previous.fee, Some(2.0));
        assert_eq!(out.previous.tvl, Some(10_000.0));
    }

    #[test]
    fn test_volume_trend_is_neutral_without_two_full_windows() {
        let history = vec![row(0, 1.0, 1_000, 3, 100_000)];
        let out = assemble(&history, Timeframe::H24).unwrap();
        assert_eq!(out.volume_trend, 0.0);
    }
}
