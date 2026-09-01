//! Orchestrates the backward signature walk, one pool at a time: page through
//! `getSignaturesForAddress`, decode each in-range transaction's events, batch-write them, and
//! checkpoint after every page so an interrupted run resumes close to where it stopped.

use std::time::Duration;

use eyre::WrapErr;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use sqlx::PgPool;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use storage::write::{NewLiquidityEvent, NewSwap, insert_liquidity_events, insert_swaps};

use crate::checkpoint::{Checkpoints, PoolCheckpoint, ResumePlan, resume_plan};
use crate::cli::Args;
use crate::convert;
use crate::range::{self, Position, RangeSpec};
use crate::rpc::HistoryClient;
use crate::{bootstrap, checkpoint};

#[derive(Default, Debug, Clone, Copy)]
pub struct CrawlSummary {
    pub pools_processed: usize,
    pub transactions_seen: u64,
    pub rows_written: u64,
}

pub async fn run(args: Args) -> eyre::Result<()> {
    tracing::info!("{args}");

    let pools = args.range.resolve_pools()?;
    let spec = args.range.spec();

    let history = HistoryClient::new(&args.rpc, &args.pacing);

    let head_slot = match spec.to_slot {
        Some(slot) => Some(slot),
        None => match history.head_slot().await {
            Ok(slot) => Some(slot),
            Err(e) => {
                tracing::warn!(error = ?e, "Could not fetch current slot; progress percentage will be unavailable");
                None
            }
        },
    };

    let db = if args.dry_run {
        None
    } else {
        let pool = args
            .postgres
            .connect()
            .await
            .wrap_err_with(|| "Connecting to Postgres")?;
        storage::run_migrations(&pool)
            .await
            .wrap_err_with(|| "Running database migrations")?;
        Some(pool)
    };

    let mut checkpoints = if args.dry_run {
        Checkpoints::default()
    } else {
        Checkpoints::load(&args.checkpoint_file).wrap_err_with(|| {
            format!("Loading checkpoint file {}", args.checkpoint_file.display())
        })?
    };

    let ct = CancellationToken::new();
    tokio::spawn({
        let ct = ct.clone();
        async move { common::shutdown_signal(ct).await }
    });

    let total_pools = pools.len();
    let mut totals = CrawlSummary::default();

    for (index, pool) in pools.iter().enumerate() {
        if ct.is_cancelled() {
            tracing::info!(
                remaining = total_pools - index,
                "Shutdown requested, stopping before remaining pools"
            );
            break;
        }

        tracing::info!(pool = %pool, index = index + 1, total = total_pools, dry_run = args.dry_run, "Starting pool backfill");

        if let Some(db) = &db {
            bootstrap::ensure_pool_row(
                db,
                &history.rpc_client(),
                args.pacing.max_concurrent_rpc,
                args.pacing.max_retries,
                pool,
            )
            .await
            .wrap_err_with(|| format!("Bootstrapping pool row for {pool}"))?;
        }

        let summary = crawl_pool(
            &history,
            db.as_ref(),
            &mut checkpoints,
            &args,
            pool,
            &spec,
            head_slot,
            &ct,
        )
        .await
        .wrap_err_with(|| format!("Crawling pool {pool}"))?;

        totals.pools_processed += 1;
        totals.transactions_seen += summary.transactions_seen;
        totals.rows_written += summary.rows_written;
    }

    tracing::info!(
        pools = totals.pools_processed,
        transactions = totals.transactions_seen,
        rows_written = totals.rows_written,
        dry_run = args.dry_run,
        "Crawl finished"
    );

    Ok(())
}

async fn flush(
    db: &PgPool,
    swaps: &mut Vec<NewSwap>,
    liquidity: &mut Vec<NewLiquidityEvent>,
) -> eyre::Result<u64> {
    if swaps.is_empty() && liquidity.is_empty() {
        return Ok(0);
    }
    let mut written = insert_swaps(db, swaps)
        .await
        .wrap_err_with(|| "Inserting swaps")?;
    written += insert_liquidity_events(db, liquidity)
        .await
        .wrap_err_with(|| "Inserting liquidity events")?;
    swaps.clear();
    liquidity.clear();
    Ok(written)
}

#[allow(clippy::too_many_arguments)]
async fn crawl_pool(
    history: &HistoryClient,
    db: Option<&PgPool>,
    checkpoints: &mut Checkpoints,
    args: &Args,
    pool: &Pubkey,
    spec: &RangeSpec,
    head_slot: Option<u64>,
    ct: &CancellationToken,
) -> eyre::Result<CrawlSummary> {
    let pool_key = pool.to_string();
    let existing = checkpoints.pools.get(&pool_key).cloned();

    let plan = if args.dry_run {
        checkpoint::ResumePlan::Fresh
    } else {
        resume_plan(existing.as_ref(), spec)
    };

    let mut before: Option<Signature> = match plan {
        ResumePlan::AlreadyComplete => {
            tracing::info!(pool = %pool, "Checkpoint already covers this range, skipping");
            return Ok(CrawlSummary {
                pools_processed: 1,
                ..Default::default()
            });
        }
        ResumePlan::Fresh => None,
        ResumePlan::Resume { before } => Some(
            before
                .parse()
                .wrap_err_with(|| format!("Parsing checkpoint cursor for {pool}"))?,
        ),
    };

    let mut transactions_seen = existing.as_ref().map(|c| c.transactions_seen).unwrap_or(0);
    let mut rows_written = existing.as_ref().map(|c| c.rows_written).unwrap_or(0);
    let mut swaps = Vec::new();
    let mut liquidity = Vec::new();

    let started = Instant::now();
    let mut complete = false;

    'paging: loop {
        let page = history
            .signatures_page(pool, before, args.range.page_size)
            .await?;
        if page.is_empty() {
            complete = true;
            break;
        }

        let mut oldest_slot_this_page = None;
        for sig_status in &page {
            oldest_slot_this_page = Some(sig_status.slot);

            if ct.is_cancelled() {
                break 'paging;
            }

            match range::classify(sig_status.slot, sig_status.block_time, spec) {
                Position::TooNew => continue,
                Position::TooOld => {
                    complete = true;
                    break 'paging;
                }
                Position::Within => {
                    if sig_status.err.is_some() {
                        // A failed transaction's inner instructions never committed, so it
                        // contributes nothing to walk -- but it still counts as covered
                        // range so progress and completion accounting stay accurate.
                        transactions_seen += 1;
                        continue;
                    }
                    transactions_seen += 1;

                    if args.dry_run {
                        continue;
                    }

                    let signature: Signature = sig_status
                        .signature
                        .parse()
                        .wrap_err_with(|| format!("Parsing signature {}", sig_status.signature))?;
                    let tx = history.transaction(&signature).await?;
                    let events = convert::decode_transaction(&tx, pool);
                    if !events.is_empty() {
                        let block_time = tx.block_time.or(sig_status.block_time).unwrap_or(0);
                        let ts = convert::unix_to_datetime(block_time);
                        for event in events {
                            convert::append_rows(
                                &pool_key,
                                ts,
                                tx.slot,
                                &sig_status.signature,
                                event,
                                &mut swaps,
                                &mut liquidity,
                            );
                        }
                    }

                    if swaps.len() + liquidity.len() >= args.range.write_batch_size {
                        let db = db.expect("db is present whenever dry_run is false");
                        rows_written += flush(db, &mut swaps, &mut liquidity).await?;
                    }
                }
            }
        }

        before = page.last().and_then(|s| s.signature.parse().ok());

        if !args.dry_run {
            save_checkpoint(
                checkpoints,
                &args.checkpoint_file,
                &pool_key,
                spec,
                before.as_ref().map(ToString::to_string),
                false,
                transactions_seen,
                rows_written,
            )?;
        }

        log_progress(
            pool,
            oldest_slot_this_page,
            spec,
            head_slot,
            transactions_seen,
            started,
        );
    }

    if let Some(db) = db {
        rows_written += flush(db, &mut swaps, &mut liquidity).await?;
    }

    if !args.dry_run {
        save_checkpoint(
            checkpoints,
            &args.checkpoint_file,
            &pool_key,
            spec,
            before.as_ref().map(ToString::to_string),
            complete,
            transactions_seen,
            rows_written,
        )?;
    }

    Ok(CrawlSummary {
        pools_processed: 1,
        transactions_seen,
        rows_written,
    })
}

#[allow(clippy::too_many_arguments)]
fn save_checkpoint(
    checkpoints: &mut Checkpoints,
    path: &std::path::Path,
    pool_key: &str,
    spec: &RangeSpec,
    cursor: Option<String>,
    complete: bool,
    transactions_seen: u64,
    rows_written: u64,
) -> eyre::Result<()> {
    checkpoints.pools.insert(
        pool_key.to_string(),
        PoolCheckpoint {
            from_slot: spec.from_slot,
            to_slot: spec.to_slot,
            from_time: spec.from_time,
            to_time: spec.to_time,
            cursor,
            complete,
            transactions_seen,
            rows_written,
        },
    );
    checkpoints
        .save(path)
        .wrap_err_with(|| format!("Saving checkpoint file {}", path.display()))
}

/// Buckets the range into roughly a hundred slices purely for a stable "N/100" progress
/// figure -- `getSignaturesForAddress` has no slot-range parameter, so this never drives a
/// call, it only turns "we're at slot S" into something a human can watch converge.
fn log_progress(
    pool: &Pubkey,
    current_slot: Option<u64>,
    spec: &RangeSpec,
    head_slot: Option<u64>,
    transactions_seen: u64,
    started: Instant,
) {
    let elapsed = started.elapsed();
    let to = spec.to_slot.or(head_slot);

    let Some(((from, to), slot)) = spec.from_slot.zip(to).zip(current_slot) else {
        tracing::info!(
            pool = %pool,
            slot = ?current_slot,
            transactions = transactions_seen,
            elapsed = ?elapsed,
            "Backfill progress (range not fully bounded; no percentage estimate available)"
        );
        return;
    };
    if to <= from {
        return;
    }

    let chunk_size = ((to - from) / 100).max(1);
    let total_chunks = range::chunk_slot_range(from, to, chunk_size).len().max(1) as u64;
    // The walk moves from `to` down towards `from`; chunk_index counts up from `from`, so
    // the chunks already covered are the ones above the current slot.
    let current_chunk = range::chunk_index(slot, from, to, chunk_size);
    let covered_chunks = total_chunks.saturating_sub(current_chunk);
    let pct = covered_chunks as f64 / total_chunks as f64 * 100.0;

    let remaining = if covered_chunks > 0 {
        let per_chunk = elapsed.as_secs_f64() / covered_chunks as f64;
        let remaining_chunks = total_chunks.saturating_sub(covered_chunks);
        humantime::format_duration(Duration::from_secs_f64(per_chunk * remaining_chunks as f64))
            .to_string()
    } else {
        "unknown".to_string()
    };

    tracing::info!(
        pool = %pool,
        slot,
        transactions = transactions_seen,
        progress_pct = format!("{pct:.1}"),
        estimated_remaining = %remaining,
        "Backfill progress"
    );
}
