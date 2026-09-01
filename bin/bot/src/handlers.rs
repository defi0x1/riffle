// Turns a parsed Command into a rendered message. This is the only module that calls into
// `storage`, and it never issues SQL of its own -- every function here is a query or write
// function that already exists in that crate, composed and formatted. The one exception is
// `dlmm_math::bin_price`/`bin_resolvable`, pure computation over an already-fetched bin step,
// not a new data source.
//
// Fund-moving commands (open/add/remove/claim/close) never build, sign or submit anything --
// there is no signing-capable type anywhere in this crate, by design (see the workspace's
// keyless guard script). What they do is: check the caller actually owns the position they
// named (or, for `open`, that they have a wallet to open one with at all), apply the same risk
// gate /potential applies before letting exposure grow, render exactly what is proposed, and
// hand back a button that deep-links into the Mini App, where the unsigned transaction is
// actually built, reviewed and signed. `DispatchOutcome` is the vehicle for that button riding
// alongside the rendered text back to `worker`. None of these commands write a
// transaction_intent from here either -- that row, and the idempotency it enforces on a
// double-tapped button, is created once by the Mini App's own backend when the button is
// actually tapped, keyed on an idempotency value the Mini App generates then. This crate's part
// of "the same idempotency handling" is simply: always route through the same deep-link
// mechanism, never a bespoke one.
use std::collections::HashSet;

use chrono::Utc;
use dlmm_math::{bin_price, bin_resolvable};
use eyre::WrapErr;
use rust_decimal::Decimal;
use sqlx::PgPool;

use storage::queries::{
    CashFlowRow, PoolRanking, PositionRow, PotentialPoolFilters, VolumeRanking, WalletRow,
    active_wallets_for_user, cash_flows_for_position, ingest_health, latest_active_bin_snapshot,
    latest_config, latest_position_valuation, latest_wallet_balances, muted_pool_addresses,
    open_positions_for_wallet, pool_detail, position_by_address, potential_pools, rationale_for,
    top_pools, volume_ranked_pools, watch_set,
};
use storage::types::{Timeframe, cash_flow_kind, venue};
use storage::write::{NewWallet, demote_pools, mute_pool, promote_pools, register_wallet};

use crate::cli::{Command, WatchAction};
use crate::mute::tag_muted;
use crate::render;
use crate::shape::looks_like_pubkey;

// Candidates are pulled well beyond what is displayed and re-sorted here by whichever metric
// the command actually ranks on -- top_pools only orders by r_org, and no query returns a
// top_score-ordered set directly. Re-sorting an already-fetched Vec is rendering logic, not a
// new query. /volume no longer needs this: volume_ranked_pools already ranks and limits.
const CANDIDATE_MULTIPLIER: i64 = 5;

// signals.kind is free TEXT (see migration 0016) rather than a typed enum in `storage`; this is
// the one value out of the four documented there ("POTENTIAL | DEGRADING | GATE_FAIL | INFO")
// that /add's risk gate treats as "cleared".
const SIGNAL_KIND_POTENTIAL: &str = "POTENTIAL";

// Everything a command needs beyond the pool connection and its own arguments. Built once per
// message in `worker` from `Config` plus whatever Telegram handed back for this update.
pub struct Context {
    pub chat_id: i64,
    // None for a message with no `from` field -- a channel post, or some anonymous-admin
    // group messages. Wallet ownership is keyed on this, so every command that touches a
    // wallet or a position needs it and refuses plainly when it is absent.
    pub telegram_user_id: Option<i64>,
    pub max_rows: usize,
    pub miniapp_base_url: reqwest::Url,
    pub max_add_value_usd: Decimal,
}

// What opens when the button under a fund-moving proposal is tapped. The bot never learns
// whether it was tapped, let alone what happened after -- the Mini App and its own backend own
// everything from here on.
pub struct MiniAppButton {
    pub label: String,
    pub url: reqwest::Url,
}

pub struct DispatchOutcome {
    pub body: String,
    pub button: Option<MiniAppButton>,
}

impl DispatchOutcome {
    pub fn text(body: String) -> Self {
        DispatchOutcome { body, button: None }
    }

    fn with_button(body: String, label: &str, url: reqwest::Url) -> Self {
        DispatchOutcome {
            body,
            button: Some(MiniAppButton {
                label: label.to_string(),
                url,
            }),
        }
    }
}

pub async fn dispatch(
    pool: &PgPool,
    ctx: &Context,
    command: Command,
) -> eyre::Result<DispatchOutcome> {
    match command {
        Command::Top { tf } => top(pool, tf.into(), ctx.max_rows)
            .await
            .map(DispatchOutcome::text),
        Command::Volume { tf } => volume(pool, tf.into(), ctx.max_rows)
            .await
            .map(DispatchOutcome::text),
        Command::Potential { tf } => potential(pool, tf.into(), ctx.chat_id)
            .await
            .map(DispatchOutcome::text),
        Command::Pool { address } => pool_cmd(pool, &address).await.map(DispatchOutcome::text),
        Command::Why { address } => why(pool, &address).await.map(DispatchOutcome::text),
        Command::Watch { address, action } => watch(pool, &address, action)
            .await
            .map(DispatchOutcome::text),
        Command::Mute { address, duration } => mute(pool, ctx.chat_id, &address, duration.into())
            .await
            .map(DispatchOutcome::text),
        Command::Status => status(pool).await.map(DispatchOutcome::text),

        Command::Wallet { pubkey, label } => wallet(pool, ctx.telegram_user_id, pubkey, label)
            .await
            .map(DispatchOutcome::text),
        Command::Balance { wallet: w } => balance(pool, ctx.telegram_user_id, w)
            .await
            .map(DispatchOutcome::text),
        Command::Positions { wallet: w } => positions(pool, ctx.telegram_user_id, w)
            .await
            .map(DispatchOutcome::text),
        Command::Profit { position } => profit(pool, ctx.telegram_user_id, &position)
            .await
            .map(DispatchOutcome::text),

        Command::Open { address, width } => open(pool, ctx, &address, width).await,
        Command::Add {
            position,
            amount_x,
            amount_y,
        } => add(pool, ctx, &position, amount_x, amount_y).await,
        Command::Remove { position, percent } => remove(pool, ctx, &position, percent).await,
        Command::Claim { position } => claim(pool, ctx, &position).await,
        Command::Close { position } => close(pool, ctx, &position).await,
    }
}

async fn top(pool: &PgPool, tf: Timeframe, max_rows: usize) -> eyre::Result<String> {
    let mut rows = top_pools(
        pool,
        venue::DLMM,
        tf,
        max_rows as i64 * CANDIDATE_MULTIPLIER,
    )
    .await
    .wrap_err_with(|| "Loading top pools")?;

    rows.sort_by(|a, b| {
        b.top_score
            .partial_cmp(&a.top_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rows.truncate(max_rows);

    Ok(render::render_top(&rows, tf))
}

async fn volume(pool: &PgPool, tf: Timeframe, max_rows: usize) -> eyre::Result<String> {
    let rows: Vec<VolumeRanking> = volume_ranked_pools(pool, venue::DLMM, tf, max_rows as i64)
        .await
        .wrap_err_with(|| "Loading volume ranking")?;

    Ok(render::render_volume(&rows, tf))
}

async fn potential(pool: &PgPool, tf: Timeframe, chat_id: i64) -> eyre::Result<String> {
    let filters = PotentialPoolFilters::default();
    let rows: Vec<PoolRanking> = potential_pools(pool, venue::DLMM, tf, &filters)
        .await
        .wrap_err_with(|| "Loading potential pools")?;

    // This is the one ranking that reads as a suggestion rather than a listing, so a muted
    // pool has to say so here -- surfacing a "worth farming" row for something the operator
    // just told the bot to stay quiet about would be worse than not checking at all.
    let muted: HashSet<String> = muted_pool_addresses(pool, chat_id)
        .await
        .wrap_err_with(|| format!("Loading muted pools for chat {chat_id}"))?
        .into_iter()
        .collect();
    let rows = tag_muted(rows, &muted, |row| &row.pool_address);

    Ok(render::render_potential(&rows, tf, filters.min_r_org))
}

async fn pool_cmd(pool: &PgPool, address: &str) -> eyre::Result<String> {
    match pool_detail(pool, address)
        .await
        .wrap_err_with(|| format!("Loading pool detail for {address}"))?
    {
        Some(detail) => Ok(render::render_pool_detail(&detail)),
        None => Ok(render::render_not_found(address)),
    }
}

async fn why(pool: &PgPool, address: &str) -> eyre::Result<String> {
    let signal = rationale_for(pool, address, Utc::now())
        .await
        .wrap_err_with(|| format!("Loading rationale for {address}"))?;

    Ok(render::render_why(address, signal.as_ref()))
}

async fn watch(pool: &PgPool, address: &str, action: Option<WatchAction>) -> eyre::Result<String> {
    let Some(detail) = pool_detail(pool, address)
        .await
        .wrap_err_with(|| format!("Loading pool detail for {address}"))?
    else {
        return Ok(render::render_not_found(address));
    };

    let now = Utc::now();
    let already_watched = detail.pool.tier == storage::types::tier::WATCHED;

    match action {
        None => {
            if already_watched {
                return Ok(render::render_watch_already_watched(address));
            }
            promote_pools(pool, &[address.to_string()], now)
                .await
                .wrap_err_with(|| format!("Promoting {address}"))?;
            Ok(render::render_watch_promoted(address))
        }
        Some(WatchAction::Off) => {
            if !already_watched {
                return Ok(render::render_watch_not_watched(address));
            }
            let demoted = demote_pools(pool, &[address.to_string()], now)
                .await
                .wrap_err_with(|| format!("Demoting {address}"))?;
            if demoted.iter().any(|a| a == address) {
                Ok(render::render_watch_released(address))
            } else {
                Ok(render::render_watch_exempt(address))
            }
        }
    }
}

async fn mute(
    pool: &PgPool,
    chat_id: i64,
    address: &str,
    duration: std::time::Duration,
) -> eyre::Result<String> {
    if pool_detail(pool, address)
        .await
        .wrap_err_with(|| format!("Loading pool detail for {address}"))?
        .is_none()
    {
        return Ok(render::render_not_found(address));
    }

    let until = Utc::now()
        + chrono::Duration::from_std(duration).unwrap_or_else(|_| chrono::Duration::zero());
    mute_pool(pool, address, chat_id, until)
        .await
        .wrap_err_with(|| format!("Muting {address} for chat {chat_id}"))?;

    Ok(render::render_mute(address, until))
}

async fn status(pool: &PgPool) -> eyre::Result<String> {
    let (ingest, watched, config) =
        tokio::try_join!(ingest_health(pool), watch_set(pool), latest_config(pool))
            .wrap_err_with(|| "Loading status")?;

    Ok(render::render_status(
        &ingest,
        watched.len(),
        config.as_ref(),
    ))
}

// `/wallet` is deliberately the only place in this crate that ever handles text a user might
// mistake for key material. `secret_guard` has already refused anything shaped like one before
// this is ever reached (see `worker::handle_message`); what lands here is either a bare list
// request or something that at least has pubkey shape. The wrap_err_with messages below are
// kept free of the pubkey itself on purpose -- see the module comment on treating a /wallet
// message as radioactive.
async fn wallet(
    pool: &PgPool,
    telegram_user_id: Option<i64>,
    pubkey: Option<String>,
    label: Option<String>,
) -> eyre::Result<String> {
    let Some(telegram_user_id) = telegram_user_id else {
        return Ok(render::render_needs_telegram_user());
    };

    match pubkey {
        None => {
            let wallets = active_wallets_for_user(pool, telegram_user_id)
                .await
                .wrap_err_with(|| "Loading registered wallets")?;
            Ok(render::render_wallet_list(&wallets))
        }
        Some(pubkey) => {
            if !looks_like_pubkey(&pubkey) {
                return Ok(render::render_wallet_invalid_pubkey());
            }
            let outcome = register_wallet(
                pool,
                &NewWallet {
                    pubkey: pubkey.clone(),
                    telegram_user_id,
                    label,
                    registered_at: Utc::now(),
                },
            )
            .await
            .wrap_err_with(|| "Registering wallet")?;
            Ok(render::render_wallet_registered(&pubkey, &outcome))
        }
    }
}

async fn caller_owns_wallet(
    pool: &PgPool,
    telegram_user_id: i64,
    wallet_address: &str,
) -> eyre::Result<bool> {
    let wallets = active_wallets_for_user(pool, telegram_user_id)
        .await
        .wrap_err_with(|| "Loading registered wallets")?;
    Ok(wallets.iter().any(|w| w.pubkey == wallet_address))
}

enum WalletTarget {
    Ready(String),
    NeedsSelection(Vec<WalletRow>),
    NoneRegistered,
    NotOwned,
}

// Shared by /balance and /positions: both take an optional wallet and fall back to "the
// caller's one wallet" when they have exactly one, since typing it every time would be
// needless friction for the common case.
async fn resolve_wallet(
    pool: &PgPool,
    telegram_user_id: i64,
    requested: Option<&str>,
) -> eyre::Result<WalletTarget> {
    let wallets = active_wallets_for_user(pool, telegram_user_id)
        .await
        .wrap_err_with(|| "Loading registered wallets")?;

    if wallets.is_empty() {
        return Ok(WalletTarget::NoneRegistered);
    }

    match requested {
        Some(address) => {
            if wallets.iter().any(|w| w.pubkey == address) {
                Ok(WalletTarget::Ready(address.to_string()))
            } else {
                Ok(WalletTarget::NotOwned)
            }
        }
        None if wallets.len() == 1 => Ok(WalletTarget::Ready(wallets[0].pubkey.clone())),
        None => Ok(WalletTarget::NeedsSelection(wallets)),
    }
}

async fn pool_pair(pool: &PgPool, pool_address: &str) -> eyre::Result<Option<(String, String)>> {
    let detail = pool_detail(pool, pool_address)
        .await
        .wrap_err_with(|| format!("Loading pool detail for {pool_address}"))?;
    Ok(detail.map(|d| (d.pool.token_x, d.pool.token_y)))
}

async fn balance(
    pool: &PgPool,
    telegram_user_id: Option<i64>,
    wallet: Option<String>,
) -> eyre::Result<String> {
    let Some(telegram_user_id) = telegram_user_id else {
        return Ok(render::render_needs_telegram_user());
    };

    match resolve_wallet(pool, telegram_user_id, wallet.as_deref()).await? {
        WalletTarget::NoneRegistered => Ok(render::render_no_wallets_registered()),
        WalletTarget::NotOwned => Ok(render::render_wallet_not_owned(
            wallet.as_deref().unwrap_or(""),
        )),
        WalletTarget::NeedsSelection(wallets) => Ok(render::render_wallets_need_selection(
            &wallets,
            "/balance <pubkey>",
        )),
        WalletTarget::Ready(address) => {
            let rows = latest_wallet_balances(pool, &address)
                .await
                .wrap_err_with(|| format!("Loading balances for wallet {address}"))?;
            Ok(render::render_balance(&address, &rows))
        }
    }
}

async fn positions(
    pool: &PgPool,
    telegram_user_id: Option<i64>,
    wallet: Option<String>,
) -> eyre::Result<String> {
    let Some(telegram_user_id) = telegram_user_id else {
        return Ok(render::render_needs_telegram_user());
    };

    match resolve_wallet(pool, telegram_user_id, wallet.as_deref()).await? {
        WalletTarget::NoneRegistered => Ok(render::render_no_wallets_registered()),
        WalletTarget::NotOwned => Ok(render::render_wallet_not_owned(
            wallet.as_deref().unwrap_or(""),
        )),
        WalletTarget::NeedsSelection(wallets) => Ok(render::render_wallets_need_selection(
            &wallets,
            "/positions <pubkey>",
        )),
        WalletTarget::Ready(address) => {
            let open = open_positions_for_wallet(pool, &address)
                .await
                .wrap_err_with(|| format!("Loading open positions for wallet {address}"))?;

            // One pool_detail lookup per open position -- no bulk "positions with pair" query
            // exists, and a caller's open-position count is small enough that this is not a
            // concerning fan-out.
            let mut rows = Vec::with_capacity(open.len());
            for position in open {
                let pair = pool_pair(pool, &position.pool_address).await?;
                rows.push((position, pair));
            }
            Ok(render::render_positions(&rows))
        }
    }
}

fn sum_cash_flow_usd(rows: &[CashFlowRow], kind: i16) -> Decimal {
    rows.iter()
        .filter(|r| r.kind == kind)
        .filter_map(|r| r.value_usd)
        .sum()
}

async fn profit(
    pool: &PgPool,
    telegram_user_id: Option<i64>,
    position_address: &str,
) -> eyre::Result<String> {
    let Some(telegram_user_id) = telegram_user_id else {
        return Ok(render::render_needs_telegram_user());
    };

    let Some(position) = position_by_address(pool, position_address)
        .await
        .wrap_err_with(|| format!("Loading position {position_address}"))?
    else {
        return Ok(render::render_position_not_found(position_address));
    };

    if !caller_owns_wallet(pool, telegram_user_id, &position.wallet_address).await? {
        return Ok(render::render_position_not_owned(position_address));
    }

    let cash_flows = cash_flows_for_position(pool, position.id)
        .await
        .wrap_err_with(|| format!("Loading cash flows for position {}", position.id))?;
    let deposits_usd = sum_cash_flow_usd(&cash_flows, cash_flow_kind::DEPOSIT);
    let realized_usd = sum_cash_flow_usd(&cash_flows, cash_flow_kind::WITHDRAWAL)
        + sum_cash_flow_usd(&cash_flows, cash_flow_kind::FEE_CLAIM);

    let valuation = latest_position_valuation(pool, position.id)
        .await
        .wrap_err_with(|| format!("Loading valuation for position {}", position.id))?;
    let pair = pool_pair(pool, &position.pool_address).await?;

    Ok(render::render_profit(
        &position,
        pair.as_ref().map(|(x, y)| (x.as_str(), y.as_str())),
        deposits_usd,
        realized_usd,
        valuation.as_ref(),
    ))
}

// Shared by every fund-moving command: does this position exist, does it belong to a wallet
// registered to whoever sent this message, and is it still open. `Err` already carries the
// fully rendered refusal -- callers just propagate it straight back out.
async fn resolve_position(
    pool: &PgPool,
    telegram_user_id: Option<i64>,
    position_address: &str,
) -> eyre::Result<Result<PositionRow, DispatchOutcome>> {
    let Some(telegram_user_id) = telegram_user_id else {
        return Ok(Err(DispatchOutcome::text(
            render::render_needs_telegram_user(),
        )));
    };

    let Some(position) = position_by_address(pool, position_address)
        .await
        .wrap_err_with(|| format!("Loading position {position_address}"))?
    else {
        return Ok(Err(DispatchOutcome::text(
            render::render_position_not_found(position_address),
        )));
    };

    if !caller_owns_wallet(pool, telegram_user_id, &position.wallet_address).await? {
        return Ok(Err(DispatchOutcome::text(
            render::render_position_not_owned(position_address),
        )));
    }

    if position.closed_at.is_some() {
        return Ok(Err(DispatchOutcome::text(render::render_position_closed(
            position_address,
        ))));
    }

    Ok(Ok(position))
}

// Builds the "review and sign" button every fund-moving proposal ends with. `action_position`
// rides as the Mini App direct link's `startapp` parameter (Telegram caps that at 64
// `[A-Za-z0-9_-]` characters; an action tag plus a base58 position address comfortably fits) --
// the Mini App re-derives everything it needs to build the transaction from there rather than
// trusting anything else this link could carry.
fn miniapp_outcome(
    ctx: &Context,
    body: String,
    action: &str,
    position_address: &str,
    label: &str,
) -> DispatchOutcome {
    let mut url = ctx.miniapp_base_url.clone();
    url.query_pairs_mut()
        .append_pair("startapp", &format!("{action}_{position_address}"));
    DispatchOutcome::with_button(body, label, url)
}

// There is no existing position to ride in the deep link the way the other four commands do
// (see `miniapp_outcome`) -- the position for `open` does not exist until the Mini App
// generates it on the user's device and the button gets tapped. The pool address plus the
// exact range this chat proposed is everything the Mini App needs to rebuild that same
// proposal and mint a fresh position for it. Worst case for the 64 `[A-Za-z0-9_-]` character
// cap Telegram enforces on `startapp` -- a 44-char base58 pool address, a 7-digit signed lower
// bin, a 2-digit width, three separating underscores plus the "open" tag -- comes to under 60.
fn miniapp_open_outcome(
    ctx: &Context,
    body: String,
    pool_address: &str,
    lower_bin_id: i32,
    width: i32,
) -> DispatchOutcome {
    let mut url = ctx.miniapp_base_url.clone();
    url.query_pairs_mut().append_pair(
        "startapp",
        &format!("open_{pool_address}_{lower_bin_id}_{width}"),
    );
    DispatchOutcome::with_button(body, "Open position", url)
}

async fn open(
    pool: &PgPool,
    ctx: &Context,
    pool_address: &str,
    width: u8,
) -> eyre::Result<DispatchOutcome> {
    let Some(telegram_user_id) = ctx.telegram_user_id else {
        return Ok(DispatchOutcome::text(render::render_needs_telegram_user()));
    };

    // Opening always needs somewhere to open into. There is no position yet to infer a wallet
    // from (unlike add/remove/claim/close), so this checks registration directly instead --
    // the Mini App resolves exactly which registered wallet is "the" one for this device, the
    // same way it already does for every other build-tx request.
    let wallets = active_wallets_for_user(pool, telegram_user_id)
        .await
        .wrap_err_with(|| "Loading registered wallets")?;
    if wallets.is_empty() {
        return Ok(DispatchOutcome::text(render::render_no_wallets_registered()));
    }

    let Some(detail) = pool_detail(pool, pool_address)
        .await
        .wrap_err_with(|| format!("Loading pool detail for {pool_address}"))?
    else {
        return Ok(DispatchOutcome::text(render::render_not_found(
            pool_address,
        )));
    };

    // The same gate /add applies before letting exposure grow in a pool -- opening a brand
    // new position is at least as consequential as adding to one that already cleared it, so
    // it cannot be the easier of the two paths into the same pool.
    let signal = rationale_for(pool, pool_address, Utc::now())
        .await
        .wrap_err_with(|| format!("Loading rationale for {pool_address}"))?;
    if signal.as_ref().map(|s| s.kind.as_str()) != Some(SIGNAL_KIND_POTENTIAL) {
        return Ok(DispatchOutcome::text(render::render_add_refused_gate(
            pool_address,
            signal.as_ref(),
        )));
    }

    // Centered on the most recently observed active bin -- the only place this bot has one on
    // record (see `latest_active_bin_snapshot`'s own doc comment). Without at least one
    // snapshot there is nothing honest to center a range on, so this refuses rather than
    // guessing a placement.
    let Some(active) = latest_active_bin_snapshot(pool, pool_address)
        .await
        .wrap_err_with(|| format!("Loading latest active bin for {pool_address}"))?
    else {
        return Ok(DispatchOutcome::text(render::render_open_no_active_bin(
            pool_address,
        )));
    };

    let width = i32::from(width);
    // Integer-divide the width around the active bin. An even width cannot be centred exactly,
    // so the extra bin lands above the active bin: width 20 on bin 1000 spans 991..=1010, nine
    // below and ten above. An arbitrary but fixed tie-break.
    // `lower <= upper` always holds by construction -- there are no two independently supplied
    // bin ids here for a caller to invert.
    let lower_bin_id = active.bin_id - (width - 1) / 2;
    let upper_bin_id = lower_bin_id + width - 1;

    // The builder itself only rejects width outside 1..=70 (already enforced at parse time);
    // this checks the range is inside what the pool's own bin step can represent at all,
    // which `width` alone cannot guarantee if the active bin sits near the edge of that
    // envelope.
    let bin_step_bps = detail.pool.bin_step as u16;
    if !bin_resolvable(lower_bin_id, bin_step_bps) || !bin_resolvable(upper_bin_id, bin_step_bps) {
        return Ok(DispatchOutcome::text(
            render::render_open_bin_range_invalid(pool_address, lower_bin_id, upper_bin_id),
        ));
    }

    let price_lower = bin_price(lower_bin_id, bin_step_bps).ok();
    let price_upper = bin_price(upper_bin_id, bin_step_bps).ok();

    let body = render::render_open_proposal(
        &detail.pool,
        lower_bin_id,
        upper_bin_id,
        width,
        price_lower,
        price_upper,
    );
    Ok(miniapp_open_outcome(
        ctx,
        body,
        pool_address,
        lower_bin_id,
        width,
    ))
}

async fn add(
    pool: &PgPool,
    ctx: &Context,
    position_address: &str,
    amount_x: Decimal,
    amount_y: Decimal,
) -> eyre::Result<DispatchOutcome> {
    let position = match resolve_position(pool, ctx.telegram_user_id, position_address).await? {
        Ok(position) => position,
        Err(outcome) => return Ok(outcome),
    };

    if amount_x <= Decimal::ZERO || amount_y <= Decimal::ZERO {
        return Ok(DispatchOutcome::text(render::render_add_invalid_amount()));
    }

    // The same gate /potential applies: adding liquidity grows exposure, so it is refused,
    // not silently allowed, against a pool that is not (or is no longer) quality-A and above
    // breakeven r_org. Remove/claim/close reduce or realize exposure instead and are not
    // gated the same way.
    let signal = rationale_for(pool, &position.pool_address, Utc::now())
        .await
        .wrap_err_with(|| format!("Loading rationale for {}", position.pool_address))?;
    if signal.as_ref().map(|s| s.kind.as_str()) != Some(SIGNAL_KIND_POTENTIAL) {
        return Ok(DispatchOutcome::text(render::render_add_refused_gate(
            &position.pool_address,
            signal.as_ref(),
        )));
    }

    let valuation = latest_position_valuation(pool, position.id)
        .await
        .wrap_err_with(|| format!("Loading valuation for position {}", position.id))?;

    if let Some((price_x, price_y)) = valuation
        .as_ref()
        .and_then(|v| v.price_x_usd.zip(v.price_y_usd))
    {
        let estimated_usd = amount_x * price_x + amount_y * price_y;
        if estimated_usd > ctx.max_add_value_usd {
            return Ok(DispatchOutcome::text(render::render_add_refused_cap(
                position_address,
                estimated_usd,
                ctx.max_add_value_usd,
            )));
        }
    }
    // No priced valuation on record yet: the cap cannot be checked from what this bot can
    // read (see the module comment on the current-price gap), so the proposal proceeds and
    // says so plainly via render_valuation_line rather than silently skipping the check.

    let pair = pool_pair(pool, &position.pool_address).await?;
    let body = render::render_add_proposal(
        &position,
        pair.as_ref().map(|(x, y)| (x.as_str(), y.as_str())),
        amount_x,
        amount_y,
        valuation.as_ref(),
    );
    Ok(miniapp_outcome(
        ctx,
        body,
        "add",
        position_address,
        "Add liquidity",
    ))
}

async fn remove(
    pool: &PgPool,
    ctx: &Context,
    position_address: &str,
    percent: u8,
) -> eyre::Result<DispatchOutcome> {
    let position = match resolve_position(pool, ctx.telegram_user_id, position_address).await? {
        Ok(position) => position,
        Err(outcome) => return Ok(outcome),
    };

    let valuation = latest_position_valuation(pool, position.id)
        .await
        .wrap_err_with(|| format!("Loading valuation for position {}", position.id))?;
    let pair = pool_pair(pool, &position.pool_address).await?;
    let body = render::render_remove_proposal(
        &position,
        pair.as_ref().map(|(x, y)| (x.as_str(), y.as_str())),
        percent,
        valuation.as_ref(),
    );
    Ok(miniapp_outcome(
        ctx,
        body,
        "remove",
        position_address,
        "Remove liquidity",
    ))
}

async fn claim(
    pool: &PgPool,
    ctx: &Context,
    position_address: &str,
) -> eyre::Result<DispatchOutcome> {
    let position = match resolve_position(pool, ctx.telegram_user_id, position_address).await? {
        Ok(position) => position,
        Err(outcome) => return Ok(outcome),
    };

    let valuation = latest_position_valuation(pool, position.id)
        .await
        .wrap_err_with(|| format!("Loading valuation for position {}", position.id))?;
    let pair = pool_pair(pool, &position.pool_address).await?;
    let body = render::render_claim_proposal(
        &position,
        pair.as_ref().map(|(x, y)| (x.as_str(), y.as_str())),
        valuation.as_ref(),
    );
    Ok(miniapp_outcome(
        ctx,
        body,
        "claim",
        position_address,
        "Claim fees",
    ))
}

async fn close(
    pool: &PgPool,
    ctx: &Context,
    position_address: &str,
) -> eyre::Result<DispatchOutcome> {
    let position = match resolve_position(pool, ctx.telegram_user_id, position_address).await? {
        Ok(position) => position,
        Err(outcome) => return Ok(outcome),
    };

    let valuation = latest_position_valuation(pool, position.id)
        .await
        .wrap_err_with(|| format!("Loading valuation for position {}", position.id))?;
    let pair = pool_pair(pool, &position.pool_address).await?;
    let body = render::render_close_proposal(
        &position,
        pair.as_ref().map(|(x, y)| (x.as_str(), y.as_str())),
        valuation.as_ref(),
    );
    Ok(miniapp_outcome(
        ctx,
        body,
        "close",
        position_address,
        "Close position",
    ))
}
