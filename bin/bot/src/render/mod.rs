mod escape;
mod paginate;

pub use escape::{escape_code_span, escape_markdown_v2};
pub use paginate::{MESSAGE_LIMIT, paginate};

use chrono::{DateTime, Utc};

use storage::queries::{
    IngestHealthStatus, LatestConfig, PoolDetail, PoolRanking, RationaleItem, SignalWithRationale,
    VolumeRanking,
};
use storage::types::{Timeframe, quality, tier};
use storage::write::IndicatorRow;

// Every helper below takes raw, unescaped text and returns something already safe to drop
// into a MarkdownV2 message. `escape_markdown_v2` is never applied a second time to their
// output, and it is never applied to text that already contains a `*`/`` ` `` marker we put
// there on purpose -- mixing the two is how a message ends up double-escaped or broken.
fn bold(text: &str) -> String {
    format!("*{}*", escape_markdown_v2(text))
}

fn code(text: &str) -> String {
    format!("`{}`", escape_code_span(text))
}

fn plain(text: &str) -> String {
    escape_markdown_v2(text)
}

fn fmt_f64(v: Option<f64>, decimals: usize) -> String {
    match v {
        Some(v) => format!("{v:.decimals$}"),
        None => "n/a".to_string(),
    }
}

fn fmt_i64(v: Option<i64>) -> String {
    v.map(|v| v.to_string())
        .unwrap_or_else(|| "n/a".to_string())
}

fn fmt_i32(v: Option<i32>) -> String {
    v.map(|v| v.to_string())
        .unwrap_or_else(|| "n/a".to_string())
}

// Generic over Decimal/etc so this module never has to name a numeric type it does not
// otherwise depend on -- it just needs whatever storage handed back to be Display.
fn fmt_opt<T: std::fmt::Display>(v: Option<T>) -> String {
    v.map(|v| v.to_string())
        .unwrap_or_else(|| "n/a".to_string())
}

fn quality_label(q: &str) -> &'static str {
    match q {
        quality::MEASURED => "A measured",
        quality::ESTIMATED => "B estimated",
        _ => "unknown quality",
    }
}

fn pair(token_x: &str, token_y: &str) -> String {
    format!("{}/{}", code(token_x), code(token_y))
}

pub fn render_refusal() -> String {
    plain("this chat is not on the allow-list, so nothing here answers to it.")
}

pub fn render_parse_error(err: &clap::Error) -> String {
    format!(
        "{}\n{}",
        bold("could not parse that command"),
        code(&err.to_string())
    )
}

pub fn render_internal_error() -> String {
    plain("that command failed unexpectedly; nothing was applied. Check the logs.")
}

pub fn render_not_found(address: &str) -> String {
    format!("no pool found at {}.", code(address))
}

// Ranked by the reproduced venue activity score, not gate-filtered -- this is "what is
// hot", not a suggestion, so it carries no threshold to state.
pub fn render_top(rows: &[PoolRanking], tf: Timeframe) -> String {
    let mut out = format!(
        "{}\n{}\n\n",
        bold(&format!("Top pools ({})", tf.as_str())),
        plain("ranked by top_score (venue activity); not gate-filtered")
    );

    if rows.is_empty() {
        out.push_str(&plain("no ranked pools for this timeframe yet."));
        return out;
    }

    for (i, row) in rows.iter().enumerate() {
        out.push_str(&format!(
            "{}. {}\n   top_score {}  r_org {}  fee/tvl {}  [{}]\n",
            i + 1,
            pair(&row.token_x, &row.token_y),
            code(&fmt_f64(row.top_score, 3)),
            code(&fmt_f64(row.r_org, 2)),
            code(&fmt_f64(row.fee_tvl, 4)),
            plain(quality_label(&row.quality)),
        ));
        out.push_str(&format!("   {}\n", code(&row.pool_address)));
    }

    out
}

// Ranked by raw volume_usd from volume_ranked_pools, with the bucket-over-bucket vol_change
// riding along in the same row -- no re-sorting and no per-row detail fetch needed here.
pub fn render_volume(rows: &[VolumeRanking], tf: Timeframe) -> String {
    let mut out = format!(
        "{}\n{}\n\n",
        bold(&format!("Volume ranking ({})", tf.as_str())),
        plain("ranked by volume; not gate-filtered")
    );

    if rows.is_empty() {
        out.push_str(&plain("no ranked pools for this timeframe yet."));
        return out;
    }

    for (i, row) in rows.iter().enumerate() {
        out.push_str(&format!(
            "{}. {}\n   volume {}  change vs previous bucket {}  [{}]\n",
            i + 1,
            pair(&row.token_x, &row.token_y),
            code(&fmt_opt(row.volume_usd)),
            code(&fmt_f64(row.vol_change, 3)),
            plain(quality_label(&row.quality)),
        ));
        out.push_str(&format!("   {}\n", code(&row.pool_address)));
    }

    out
}

// The one ranking that is a suggestion rather than a listing, so every row states the
// threshold it cleared: r_org above the same min_r_org the query itself filtered on. A muted
// pool still appears -- muting suppresses alerts, not the ranking itself -- but is tagged so
// a suggestion for something the operator asked to stay quiet about cannot be mistaken for
// an active recommendation.
pub fn render_potential(rows: &[(PoolRanking, bool)], tf: Timeframe, min_r_org: f64) -> String {
    let mut out = format!(
        "{}\n{}\n\n",
        bold(&format!("Potential ({})", tf.as_str())),
        plain("gate-filtered: quality-A only, r_org at or above breakeven")
    );

    if rows.is_empty() {
        out.push_str(&plain(
            "nothing clears the gate right now. Try /why on a specific pool to see the closest misses.",
        ));
        return out;
    }

    for (i, (row, muted)) in rows.iter().enumerate() {
        let muted_tag = if *muted { "  [MUTED]" } else { "" };
        out.push_str(&format!(
            "{}. {}\n   r_org {} >= {} (breakeven {})  fee/tvl {}  [{}]{}\n",
            i + 1,
            pair(&row.token_x, &row.token_y),
            code(&fmt_f64(row.r_org, 2)),
            code(&format!("{min_r_org:.2}")),
            code("1.0"),
            code(&fmt_f64(row.fee_tvl, 4)),
            plain(quality_label(&row.quality)),
            plain(muted_tag),
        ));
        out.push_str(&format!("   {}\n", code(&row.pool_address)));
    }

    out
}

pub fn render_pool_detail(detail: &PoolDetail) -> String {
    let p = &detail.pool;
    let tier_label = if p.tier == tier::WATCHED {
        "watched"
    } else {
        "universe"
    };

    let summary = format!(
        "{}  bin_step {}  base fee {} bps  tier {}  tvl {}",
        pair(&p.token_x, &p.token_y),
        code(&p.bin_step.to_string()),
        code(&p.base_fee_bps.to_string()),
        code(tier_label),
        code(
            &p.tvl_usd
                .map(|v| v.to_string())
                .unwrap_or_else(|| "n/a".to_string())
        ),
    );
    let mut out = format!(
        "{}\n{}\n{}\n\n",
        bold("Pool"),
        code(&p.pool_address),
        summary
    );

    for (label, row) in [
        ("5m", &detail.m5),
        ("10m", &detail.m10),
        ("1h", &detail.h1),
        ("4h", &detail.h4),
        ("24h", &detail.h24),
    ] {
        out.push_str(&render_indicator_line(label, row));
    }

    out
}

fn render_indicator_line(label: &str, row: &Option<IndicatorRow>) -> String {
    match row {
        None => format!("{}: no data yet\n", bold(label)),
        Some(r) => format!(
            "{} [{}]: r_org {}  fee/tvl {}  vol/tvl {}  regime {}\n",
            bold(label),
            plain(quality_label(&r.quality)),
            code(&fmt_f64(r.r_org, 2)),
            code(&fmt_f64(r.fee_tvl, 4)),
            code(&fmt_f64(r.vol_tvl, 4)),
            code(r.regime.as_deref().unwrap_or("n/a")),
        ),
    }
}

// /why has to explain silence as well as noise: a pool with no signal at all did not fail a
// threshold, it was simply never scored -- a materially different statement, so it gets a
// different message rather than being folded into "no rationale".
pub fn render_why(address: &str, signal: Option<&SignalWithRationale>) -> String {
    let mut out = format!("{} {}\n\n", bold("Why"), code(address));

    let Some(signal) = signal else {
        out.push_str(&plain(
            "no evaluation on record for this pool. Either it has never been promoted to \
             measured status, or scoring has not run since it was -- a screening-only \
             (quality B) pool is never gated into a signal.",
        ));
        return out;
    };

    out.push_str(&format!(
        "kind {}  timeframe {}  regime {}\n\n",
        code(&signal.kind),
        code(&signal.timeframe),
        code(signal.regime.as_deref().unwrap_or("n/a")),
    ));

    if signal.items.is_empty() {
        out.push_str(&plain(
            "no evaluated conditions were recorded for this signal.",
        ));
        return out;
    }

    for item in &signal.items {
        out.push_str(&render_rationale_line(item));
    }

    out
}

fn render_rationale_line(item: &RationaleItem) -> String {
    let mark = if item.passed { "PASS" } else { "FAIL" };
    let mut line = format!(
        "{} {}: observed {} {} threshold {}",
        mark,
        plain(&item.signal),
        code(item.observed.as_deref().unwrap_or("n/a")),
        code(item.cmp.as_deref().unwrap_or("cmp")),
        code(item.threshold.as_deref().unwrap_or("n/a")),
    );
    if let Some(note) = &item.note {
        line.push_str(&format!(" note: {}", plain(note)));
    }
    line.push('\n');
    line
}

pub fn render_watch_promoted(address: &str) -> String {
    format!(
        "{} is now watched (tier-1, measured indicators from the next tick).",
        code(address)
    )
}

pub fn render_watch_already_watched(address: &str) -> String {
    format!("{} is already watched.", code(address))
}

pub fn render_watch_not_watched(address: &str) -> String {
    format!(
        "{} is not currently watched (nothing to release).",
        code(address)
    )
}

pub fn render_watch_released(address: &str) -> String {
    format!("{} released back to screening-only.", code(address))
}

pub fn render_watch_exempt(address: &str) -> String {
    format!(
        "{} has an open paper position and stays watched until it closes -- releasing it now \
         would corrupt the measurement in progress.",
        code(address)
    )
}

pub fn render_mute(address: &str, until: DateTime<Utc>) -> String {
    format!(
        "{} muted until {} UTC.",
        code(address),
        code(&until.format("%Y-%m-%d %H:%M").to_string()),
    )
}

// config_hash is stamped on every signal at write time; latest_config surfaces the newest one
// across all pools as "what configuration is currently applied".
pub fn render_status(
    ingest: &[IngestHealthStatus],
    tier_size: usize,
    config: Option<&LatestConfig>,
) -> String {
    let mut out = format!("{}\n\n", bold("Status"));

    match config {
        Some(c) => out.push_str(&format!(
            "config: {} (as of {})\n",
            code(&c.config_hash),
            code(&c.ts.format("%Y-%m-%d %H:%M:%S").to_string()),
        )),
        None => out.push_str(&plain("config: no signals recorded yet.\n")),
    }

    out.push_str(&format!(
        "watched pools: {}\n\n",
        code(&tier_size.to_string())
    ));

    if ingest.is_empty() {
        out.push_str(&plain("no ingest health rows recorded yet."));
        return out;
    }

    for row in ingest {
        out.push_str(&format!(
            "{}: last_slot {}  slot_gap {}  decode_errors {}  write_latency {} ms  at {}\n",
            plain(&row.source),
            code(&fmt_i64(row.last_slot)),
            code(&fmt_i64(row.slot_gap)),
            code(&fmt_i32(row.decode_errors)),
            code(&fmt_i32(row.write_latency_ms)),
            code(&row.ts.format("%Y-%m-%d %H:%M:%S").to_string()),
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_top_empty_states_it_plainly() {
        let out = render_top(&[], Timeframe::M5);
        assert!(out.contains("no ranked pools"));
    }

    #[test]
    fn test_render_why_explains_silence_when_no_signal_exists() {
        let out = render_why("addr1", None);
        assert!(out.to_lowercase().contains("no evaluation on record"));
    }

    #[test]
    fn test_render_watch_exempt_mentions_open_position() {
        let out = render_watch_exempt("addr1");
        assert!(out.contains("open paper position"));
    }

    #[test]
    fn test_bold_escapes_special_characters_in_header() {
        let out = bold("r_org (breakeven 1.0)");
        assert!(out.starts_with('*'));
        assert!(out.ends_with('*'));
        assert!(out.contains("r\\_org"));
        assert!(out.contains("\\(breakeven 1\\.0\\)"));
    }
}
