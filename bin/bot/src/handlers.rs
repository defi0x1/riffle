// Turns a parsed Command into a rendered message. This is the only module that calls into
// `storage`, and it never issues SQL of its own -- every function here is a query or write
// function that already exists in that crate, composed and formatted.
use chrono::Utc;
use eyre::WrapErr;
use sqlx::PgPool;

use storage::queries::{
    PoolRanking, PotentialPoolFilters, ingest_health, pool_detail, potential_pools, rationale_for,
    top_pools, watch_set,
};
use storage::types::{Timeframe, venue};
use storage::write::{IndicatorRow, demote_pools, promote_pools};

use crate::cli::{Command, WatchAction};
use crate::mute::MuteStore;
use crate::render;

// Candidates are pulled well beyond what is displayed and re-sorted here by whichever metric
// the command actually ranks on -- top_pools only orders by r_org, and no query returns a
// vol_tvl- or top_score-ordered set directly. Re-sorting a already-fetched Vec is rendering
// logic, not a new query.
const CANDIDATE_MULTIPLIER: i64 = 5;

pub async fn dispatch(
    pool: &PgPool,
    mutes: &MuteStore,
    max_rows: usize,
    command: Command,
) -> eyre::Result<String> {
    match command {
        Command::Top { tf } => top(pool, tf.into(), max_rows).await,
        Command::Volume { tf } => volume(pool, tf.into(), max_rows).await,
        Command::Potential { tf } => potential(pool, tf.into(), mutes).await,
        Command::Pool { address } => pool_cmd(pool, &address).await,
        Command::Why { address } => why(pool, &address).await,
        Command::Watch { address, action } => watch(pool, &address, action).await,
        Command::Mute { address, duration } => mute(pool, mutes, &address, duration.into()).await,
        Command::Status => status(pool).await,
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
    let mut rows = top_pools(
        pool,
        venue::DLMM,
        tf,
        max_rows as i64 * CANDIDATE_MULTIPLIER,
    )
    .await
    .wrap_err_with(|| "Loading volume candidates")?;

    rows.sort_by(|a, b| {
        b.vol_tvl
            .partial_cmp(&a.vol_tvl)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rows.truncate(max_rows);

    // vol_change lives on indicators_{tf}, not on the flat PoolRanking row that top_pools
    // returns, so it takes one pool_detail call per displayed row. Bounded by max_rows, so
    // this stays well inside the per-command latency budget.
    let mut with_change = Vec::with_capacity(rows.len());
    for row in rows {
        let change = pool_detail(pool, &row.pool_address)
            .await
            .ok()
            .flatten()
            .and_then(|detail| indicator_for(&detail, tf).and_then(|i| i.vol_change));
        with_change.push((row, change));
    }

    Ok(render::render_volume(&with_change, tf))
}

fn indicator_for(detail: &storage::queries::PoolDetail, tf: Timeframe) -> Option<&IndicatorRow> {
    match tf {
        Timeframe::M5 => detail.m5.as_ref(),
        Timeframe::M10 => detail.m10.as_ref(),
        Timeframe::H1 => detail.h1.as_ref(),
        Timeframe::H4 => detail.h4.as_ref(),
        Timeframe::H24 => detail.h24.as_ref(),
    }
}

async fn potential(pool: &PgPool, tf: Timeframe, mutes: &MuteStore) -> eyre::Result<String> {
    let filters = PotentialPoolFilters::default();
    let rows: Vec<PoolRanking> = potential_pools(pool, venue::DLMM, tf, &filters)
        .await
        .wrap_err_with(|| "Loading potential pools")?;

    // This is the one ranking that reads as a suggestion rather than a listing, so a muted
    // pool has to say so here -- surfacing a "worth farming" row for something the operator
    // just told the bot to stay quiet about would be worse than not checking at all.
    let rows: Vec<(PoolRanking, bool)> = rows
        .into_iter()
        .map(|row| {
            let muted = mutes.is_muted(&row.pool_address);
            (row, muted)
        })
        .collect();

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
    mutes: &MuteStore,
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
    mutes.mute(address.to_string(), until);

    Ok(render::render_mute(address, until))
}

async fn status(pool: &PgPool) -> eyre::Result<String> {
    let (ingest, watched) = tokio::try_join!(ingest_health(pool), watch_set(pool))
        .wrap_err_with(|| "Loading status")?;

    Ok(render::render_status(&ingest, watched.len()))
}
