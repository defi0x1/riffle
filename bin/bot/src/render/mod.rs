mod escape;
mod paginate;

pub use escape::{escape_code_span, escape_markdown_v2};
pub use paginate::{MESSAGE_LIMIT, paginate};

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use storage::queries::{
    IngestHealthStatus, LatestConfig, PoolDetail, PoolRanking, PositionRow, PositionValuationRow,
    RationaleItem, SignalWithRationale, VolumeRanking, WalletBalanceRow, WalletRow,
};
use storage::types::{Timeframe, quality, tier};
use storage::write::{IndicatorRow, RegisterWalletOutcome};

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

// A fund-moving proposal always ends with the same statement of fact: this chat cannot sign
// anything, and nothing moves until the Mini App is used. Centralised so every proposal states
// it identically rather than in slightly different words each time.
fn miniapp_notice() -> String {
    plain(
        "this chat cannot sign anything. Tap the button below to review and sign this in the \
         Mini App -- nothing moves until you approve it there, and you will be sent back here \
         once it confirms.",
    )
}

fn fmt_decimal(v: Decimal) -> String {
    v.normalize().to_string()
}

fn fmt_opt_decimal(v: Option<Decimal>) -> String {
    v.map(fmt_decimal).unwrap_or_else(|| "n/a".to_string())
}

// The one place a position's live-priced state gets rendered from, since three of the four
// fund-moving proposals (and /profit) all want the same "what is this worth right now" line.
// `position_valuations` is only ever populated once a marking pass has run for the position, so
// `None` is a real, fairly common state (a just-opened position, most obviously) rather than a
// query failure -- rendered plainly rather than omitted, so a missing price is never mistaken
// for a zero one.
fn render_valuation_line(valuation: Option<&PositionValuationRow>) -> String {
    match valuation {
        None => plain(
            "no live valuation on record yet for this position -- current price and value will \
             be shown in the Mini App before you confirm.",
        ),
        Some(v) => format!(
            "as of {}: price {} / {}  value {}  in range {}\n",
            code(&v.ts.format("%Y-%m-%d %H:%M:%S").to_string()),
            code(&fmt_opt_decimal(v.price_x_usd)),
            code(&fmt_opt_decimal(v.price_y_usd)),
            code(&fmt_opt_decimal(v.value_usd)),
            code(match v.in_range {
                Some(true) => "yes",
                Some(false) => "no",
                None => "n/a",
            }),
        ),
    }
}

fn position_header(position: &PositionRow, token_pair: Option<(&str, &str)>) -> String {
    let pair_label = match token_pair {
        Some((x, y)) => pair(x, y),
        None => plain("unknown pair"),
    };
    format!(
        "{}\n{}\n{}\nbin range {}\n",
        bold("Position"),
        code(&position.position_address),
        pair_label,
        code(&format!("{} - {}", position.lower_bin, position.upper_bin)),
    )
}

pub fn render_key_material_refusal() -> String {
    format!(
        "{}\n\n{}",
        bold("refused -- this looked like a private key or seed phrase"),
        plain(
            "/wallet only ever accepts a public key; signing happens on your own device inside \
             the Mini App, and this bot and its backend never see, store, or log a private key. \
             I have not stored or logged what you just sent, but Telegram has already kept a \
             copy of it in this chat's history -- delete that message now, and treat whatever \
             key or phrase it contained as compromised: abandon it, and create or import a new \
             wallet in the Mini App instead.",
        ),
    )
}

pub fn render_needs_telegram_user() -> String {
    plain(
        "this command needs your Telegram user identity, which was not present on this \
         message (channel posts and some anonymous-admin messages do not carry one) -- send it \
         as yourself in a normal message.",
    )
}

pub fn render_wallet_invalid_pubkey() -> String {
    plain(
        "that does not look like a Solana public key (expected a base58 string, 32-44 \
         characters). If you meant to paste a private key or seed phrase, stop -- /wallet \
         never needs one; register the public key shown in the Mini App instead.",
    )
}

pub fn render_wallet_registered(pubkey: &str, outcome: &RegisterWalletOutcome) -> String {
    match outcome {
        RegisterWalletOutcome::Registered => {
            format!("{} {}", code(pubkey), plain("registered to your account."))
        }
        RegisterWalletOutcome::AlreadyOwnedByCaller => {
            format!(
                "{} {}",
                code(pubkey),
                plain("is already yours -- label refreshed.")
            )
        }
        RegisterWalletOutcome::OwnedByAnotherUser { .. } => format!(
            "{} {}",
            code(pubkey),
            plain(
                "is already registered to a different Telegram account. Its owner has to \
                 revoke it before it can be registered here.",
            )
        ),
    }
}

pub fn render_wallet_list(wallets: &[WalletRow]) -> String {
    if wallets.is_empty() {
        return render_no_wallets_registered();
    }

    let mut out = format!("{}\n\n", bold("Your wallets"));
    for w in wallets {
        let label = w.label.as_deref().unwrap_or("(no label)");
        out.push_str(&format!(
            "{}{}{}{}{}\n",
            code(&w.pubkey),
            plain(" -- "),
            plain(label),
            plain(", registered "),
            code(&w.registered_at.format("%Y-%m-%d").to_string()),
        ));
    }
    out
}

pub fn render_no_wallets_registered() -> String {
    plain(
        "no wallet registered yet. Send /wallet <pubkey> with the public key shown in the Mini App.",
    )
}

pub fn render_wallet_not_owned(pubkey: &str) -> String {
    format!(
        "{} {}",
        code(pubkey),
        plain(
            "is not registered to you. Register it first with /wallet, or use one of your own \
             registered wallets.",
        )
    )
}

pub fn render_wallets_need_selection(wallets: &[WalletRow], example: &str) -> String {
    let mut out = format!(
        "{}\n\n",
        plain("you have more than one registered wallet -- say which one:")
    );
    for w in wallets {
        out.push_str(&format!("{}\n", code(&w.pubkey)));
    }
    out.push_str(&format!("\n{} {}", plain("e.g."), code(example)));
    out
}

pub fn render_balance(wallet: &str, rows: &[WalletBalanceRow]) -> String {
    let mut out = format!("{} {}\n\n", bold("Balances for"), code(wallet));
    if rows.is_empty() {
        out.push_str(&plain(
            "no balances on record yet -- they refresh on a fixed poll cadence once a wallet \
             is registered.",
        ));
        return out;
    }
    for r in rows {
        out.push_str(&format!(
            "{}  {}  {}\n",
            code(&r.mint),
            code(&fmt_decimal(r.amount)),
            code(&fmt_opt_decimal(r.value_usd)),
        ));
    }
    out
}

pub fn render_positions(rows: &[(PositionRow, Option<(String, String)>)]) -> String {
    if rows.is_empty() {
        return plain("no open positions for this wallet.");
    }

    let mut out = format!("{}\n\n", bold("Open positions"));
    for (p, pair_opt) in rows {
        out.push_str(&position_header(
            p,
            pair_opt.as_ref().map(|(x, y)| (x.as_str(), y.as_str())),
        ));
        out.push_str(&format!(
            "opened {}\n\n",
            code(&p.opened_at.format("%Y-%m-%d %H:%M").to_string()),
        ));
    }
    out
}

pub fn render_position_not_found(address: &str) -> String {
    format!("{} {}", plain("no position found at"), code(address))
}

pub fn render_position_not_owned(address: &str) -> String {
    format!(
        "{} {}",
        code(address),
        plain("does not belong to a wallet registered to you.")
    )
}

pub fn render_position_closed(address: &str) -> String {
    format!(
        "{} {}",
        code(address),
        plain("is already closed -- there is nothing left to act on.")
    )
}

pub fn render_profit(
    position: &PositionRow,
    pair: Option<(&str, &str)>,
    deposits_usd: Decimal,
    realized_usd: Decimal,
    valuation: Option<&PositionValuationRow>,
) -> String {
    let mut out = position_header(position, pair);
    out.push('\n');

    let unrealized_usd = valuation.and_then(|v| v.value_usd).unwrap_or(Decimal::ZERO);
    let profit_usd = realized_usd + unrealized_usd - deposits_usd;

    out.push_str(&format!(
        "{}{}{}{}{}{}\n",
        plain("deposited "),
        code(&fmt_decimal(deposits_usd)),
        plain("  withdrawn/claimed "),
        code(&fmt_decimal(realized_usd)),
        plain("  current value "),
        code(&fmt_opt_decimal(valuation.and_then(|v| v.value_usd))),
    ));
    out.push_str(&format!(
        "{} {}\n",
        bold("profit (USD, realized + unrealized):"),
        code(&fmt_decimal(profit_usd)),
    ));

    if let Some(hold_usd) = valuation.and_then(|v| v.hold_value_usd) {
        let vs_hold = unrealized_usd - hold_usd;
        out.push_str(&format!(
            "{} {} {}\n",
            bold("vs. holding:"),
            code(&fmt_decimal(vs_hold)),
            plain("(value now vs. simply holding the deposited tokens)"),
        ));
    }

    out.push('\n');
    out.push_str(&render_valuation_line(valuation));
    out
}

pub fn render_add_refused_gate(pool_address: &str, signal: Option<&SignalWithRationale>) -> String {
    let mut out = format!(
        "{}\n{}\n\n",
        bold("refused -- this pool does not currently clear the risk gate"),
        code(pool_address),
    );

    let Some(signal) = signal else {
        out.push_str(&plain(
            "no evaluation on record for this pool yet, so it cannot be verified against the \
             gate -- try again once scoring has run for it.",
        ));
        return out;
    };

    if signal.kind == "POTENTIAL" {
        // Should not happen if the caller checked `kind` first, but stated plainly rather than
        // silently proceeding if this function is ever called on a passing pool.
        out.push_str(&plain("this pool currently clears the gate."));
        return out;
    }

    out.push_str(&format!(
        "{}{}{}{}\n\n",
        plain("latest evaluation: "),
        code(&signal.kind),
        plain(", timeframe "),
        code(&signal.timeframe),
    ));
    for item in &signal.items {
        out.push_str(&render_rationale_line(item));
    }
    out
}

pub fn render_add_refused_cap(
    position_address: &str,
    estimated_usd: Decimal,
    cap_usd: Decimal,
) -> String {
    format!(
        "{}\n{}\n{}{}{}{}{}",
        bold("refused -- over the per-transaction cap"),
        code(position_address),
        plain("estimated value "),
        code(&fmt_decimal(estimated_usd)),
        plain(" exceeds the configured cap "),
        code(&fmt_decimal(cap_usd)),
        plain(" -- split this into smaller adds, or ask an operator to raise the configured cap."),
    )
}

pub fn render_add_invalid_amount() -> String {
    plain("both amounts must be greater than zero.")
}

pub fn render_add_proposal(
    position: &PositionRow,
    pair: Option<(&str, &str)>,
    amount_x: Decimal,
    amount_y: Decimal,
    valuation: Option<&PositionValuationRow>,
) -> String {
    let mut out = position_header(position, pair);
    out.push_str(&format!(
        "\n{} add {} / {}\nstrategy {}\n\n",
        bold("proposed:"),
        code(&fmt_decimal(amount_x)),
        code(&fmt_decimal(amount_y)),
        code("SpotBalanced"),
    ));
    out.push_str(&render_valuation_line(valuation));
    out.push('\n');
    out.push_str(&miniapp_notice());
    out
}

pub fn render_remove_proposal(
    position: &PositionRow,
    pair: Option<(&str, &str)>,
    percent: u8,
    valuation: Option<&PositionValuationRow>,
) -> String {
    let mut out = position_header(position, pair);
    out.push_str(&format!(
        "\n{} withdraw {} of this position\n\n",
        bold("proposed:"),
        code(&format!("{percent}%")),
    ));
    out.push_str(&render_valuation_line(valuation));
    out.push('\n');
    out.push_str(&miniapp_notice());
    out
}

pub fn render_claim_proposal(
    position: &PositionRow,
    pair: Option<(&str, &str)>,
    valuation: Option<&PositionValuationRow>,
) -> String {
    let mut out = position_header(position, pair);
    out.push_str(&format!("\n{}\n\n", bold("proposed: claim accrued fees")));
    if let Some(v) = valuation {
        out.push_str(&format!(
            "uncollected fees: {} / {}\n\n",
            code(&fmt_opt_decimal(v.fees_x_uncollected)),
            code(&fmt_opt_decimal(v.fees_y_uncollected)),
        ));
    }
    out.push_str(&render_valuation_line(valuation));
    out.push('\n');
    out.push_str(&miniapp_notice());
    out
}

pub fn render_close_proposal(
    position: &PositionRow,
    pair: Option<(&str, &str)>,
    valuation: Option<&PositionValuationRow>,
) -> String {
    let mut out = position_header(position, pair);
    out.push_str(&format!(
        "\n{}\n\n",
        bold("proposed: withdraw everything and close this position"),
    ));
    out.push_str(&render_valuation_line(valuation));
    out.push('\n');
    out.push_str(&miniapp_notice());
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

    fn sample_position() -> PositionRow {
        PositionRow {
            id: uuid::Uuid::nil(),
            position_address: "position_addr_1".to_string(),
            wallet_address: "wallet_addr_1".to_string(),
            pool_address: "pool_addr_1".to_string(),
            venue: storage::types::venue::DLMM,
            opened_at: Utc::now(),
            entry_active_bin: Some(100),
            lower_bin: 90,
            upper_bin: 110,
            closed_at: None,
            close_reason: None,
        }
    }

    #[test]
    fn test_render_key_material_refusal_never_takes_the_message_as_input() {
        // The strongest guarantee against echoing a key back to the chat is a function that
        // structurally cannot receive the raw text in the first place.
        let out = render_key_material_refusal();
        assert!(out.to_lowercase().contains("private key or seed phrase"));
        assert!(out.to_lowercase().contains("not stored or logged"));
        assert!(out.to_lowercase().contains("compromised"));
    }

    #[test]
    fn test_render_add_proposal_states_pool_amounts_strategy_and_the_miniapp_step() {
        let position = sample_position();
        let out = render_add_proposal(
            &position,
            Some(("SOL", "USDC")),
            Decimal::new(15, 1),
            Decimal::new(2250, 2),
            None,
        );
        assert!(out.contains("position_addr_1"));
        assert!(out.contains("SOL"));
        assert!(out.contains("USDC"));
        assert!(out.contains("1.5"));
        assert!(out.contains("22.5"));
        assert!(out.contains("SpotBalanced"));
        assert!(out.to_lowercase().contains("mini app"));
        assert!(out.to_lowercase().contains("cannot sign"));
    }

    #[test]
    fn test_render_add_proposal_notes_missing_live_valuation() {
        let out = render_add_proposal(
            &sample_position(),
            Some(("SOL", "USDC")),
            Decimal::ONE,
            Decimal::ONE,
            None,
        );
        assert!(out.to_lowercase().contains("no live valuation"));
    }

    #[test]
    fn test_render_remove_proposal_states_the_percentage() {
        let out = render_remove_proposal(&sample_position(), Some(("SOL", "USDC")), 25, None);
        assert!(out.contains("25%"));
        assert!(out.to_lowercase().contains("mini app"));
    }

    #[test]
    fn test_render_close_proposal_states_full_withdrawal() {
        let out = render_close_proposal(&sample_position(), Some(("SOL", "USDC")), None);
        assert!(out.to_lowercase().contains("close"));
        assert!(out.to_lowercase().contains("mini app"));
    }

    #[test]
    fn test_render_add_refused_cap_explains_the_numbers() {
        let out = render_add_refused_cap(
            "position_addr_1",
            Decimal::new(9_000, 0),
            Decimal::new(5_000, 0),
        );
        assert!(out.contains("9000"));
        assert!(out.contains("5000"));
        assert!(out.to_lowercase().contains("cap"));
    }

    #[test]
    fn test_render_add_refused_gate_explains_a_gate_fail_with_rationale() {
        let signal = SignalWithRationale {
            id: uuid::Uuid::nil(),
            ts: Utc::now(),
            pool_address: "pool_addr_1".to_string(),
            venue: storage::types::venue::DLMM,
            timeframe: "5m".to_string(),
            kind: "GATE_FAIL".to_string(),
            regime: None,
            items: vec![RationaleItem {
                signal_id: uuid::Uuid::nil(),
                seq: 0,
                venue: storage::types::venue::DLMM,
                signal: "r_org".to_string(),
                observed: Some("0.8".to_string()),
                cmp: Some(">=".to_string()),
                threshold: Some("1.0".to_string()),
                passed: false,
                note: None,
            }],
        };
        let out = render_add_refused_gate("pool_addr_1", Some(&signal));
        assert!(out.to_lowercase().contains("risk gate"));
        assert!(out.contains("GATE_FAIL"));
        assert!(out.contains("FAIL"));
    }

    #[test]
    fn test_render_add_refused_gate_explains_missing_evaluation() {
        let out = render_add_refused_gate("pool_addr_1", None);
        assert!(out.to_lowercase().contains("no evaluation on record"));
    }

    #[test]
    fn test_render_wallet_not_owned_explains_itself() {
        let out = render_wallet_not_owned("wallet_addr_1");
        assert!(out.to_lowercase().contains("not registered to you"));
    }

    #[test]
    fn test_render_position_not_owned_explains_itself() {
        let out = render_position_not_owned("position_addr_1");
        assert!(out.to_lowercase().contains("does not belong"));
    }

    #[test]
    fn test_render_no_wallets_registered_points_at_wallet_command() {
        let out = render_no_wallets_registered();
        assert!(out.contains("/wallet"));
    }

    #[test]
    fn test_render_wallet_registered_reports_a_conflict_without_naming_the_owner() {
        let out = render_wallet_registered(
            "pubkey1",
            &RegisterWalletOutcome::OwnedByAnotherUser {
                owner_telegram_user_id: 12345,
            },
        );
        assert!(!out.contains("12345"));
        assert!(out.to_lowercase().contains("different telegram account"));
    }
}
