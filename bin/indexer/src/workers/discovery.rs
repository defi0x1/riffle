use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use eyre::WrapErr;
use futures::StreamExt;
use rust_decimal::Decimal;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;

use common::{Worker, tick_loop};
use source::{Source, WatchSet};
use storage::queries::{PoolForScoring, scoring_universe};
use storage::write::{NewPool, upsert_dlmm_pool, upsert_pool};

use crate::convert::{PoolMetadata, pool_rows, unix_to_datetime};

/// Full-universe pool discovery. Every discovery tick diffs the chain's own pool set against
/// what `pools` already holds (`scoring_universe`): an address on-chain but not in the
/// database is onboarded with a one-off full account read (mints and static parameters,
/// which a zero-slice `getProgramAccounts` scan cannot supply); an address already known has
/// its `tvl_usd`/`is_blacklisted`/`launchpad` cache refreshed from the flow-metrics source.
/// An address in the database but no longer seen on-chain is logged, not deleted -- see the
/// warning below.
pub struct DiscoveryWorker {
    pool: PgPool,
    source: Arc<dyn Source>,
    interval: std::time::Duration,
    batch_size: usize,
}

impl DiscoveryWorker {
    pub fn new(
        pool: PgPool,
        source: Arc<dyn Source>,
        interval: std::time::Duration,
        batch_size: usize,
    ) -> Self {
        Self {
            pool,
            source,
            interval,
            batch_size,
        }
    }

    async fn onboard_new_pools(
        &self,
        addresses: Vec<solana_sdk::pubkey::Pubkey>,
    ) -> eyre::Result<usize> {
        if addresses.is_empty() {
            return Ok(0);
        }

        let watch_set = WatchSet {
            pools: addresses.clone(),
        };
        let mut stream = self.source.state_stream(watch_set).take(addresses.len());

        let mut onboarded = 0usize;
        while let Some(update) = stream.next().await {
            let Some(state) = update.lb_pair else {
                tracing::warn!(pool = %update.pool, "New pool has no decodable LbPair yet, skipping");
                continue;
            };

            let meta = PoolMetadata {
                tvl_usd: None,
                is_blacklisted: false,
                launchpad: None,
                created_at: unix_to_datetime(update.block_time),
                updated_at: Utc::now(),
            };
            let (shared, params) = pool_rows(&update.pool, &state, &meta)?;

            upsert_dlmm_pool(&self.pool, &shared, &params)
                .await
                .wrap_err_with(|| format!("Onboarding pool {}", update.pool))?;

            onboarded += 1;
        }

        Ok(onboarded)
    }

    async fn refresh_known_pools(
        &self,
        existing: &HashMap<String, PoolForScoring>,
    ) -> eyre::Result<usize> {
        let addresses: Vec<solana_sdk::pubkey::Pubkey> = existing
            .keys()
            .filter_map(|addr| addr.parse().ok())
            .collect();
        if addresses.is_empty() {
            return Ok(0);
        }

        let Some(flow) = self
            .source
            .flow_metrics(&addresses)
            .await
            .wrap_err_with(|| "Fetching flow metrics for known pools")?
        else {
            // Geyser computes its own flow metrics from decoded events; nothing to refresh here.
            return Ok(0);
        };

        let now = Utc::now();
        let mut refreshed = 0usize;
        for row in flow {
            let address = row.pool.to_string();
            let Some(prior) = existing.get(&address) else {
                continue;
            };

            // Everything not overwritten here (mints, base fee, protocol share, activation,
            // timestamps) is preserved from the row already on disk -- this refresh only
            // touches the fields the flow-metrics source is actually authoritative for.
            let refreshed_row = NewPool {
                pool_address: address,
                venue: prior.venue,
                token_x: prior.token_x.clone(),
                token_y: prior.token_y.clone(),
                base_fee_bps: prior.base_fee_bps,
                protocol_share_bps: prior.protocol_share_bps,
                tvl_usd: Decimal::from_f64_retain(row.tvl),
                status: prior.status,
                creator: None,
                activation_point: prior.activation_point,
                created_at: prior.created_at,
                first_liquidity_at: prior.first_liquidity_at,
                is_blacklisted: row.is_blacklisted,
                launchpad: row.launchpad.clone(),
                tags: Vec::new(),
                updated_at: now,
            };

            upsert_pool(&self.pool, &refreshed_row)
                .await
                .wrap_err_with(|| format!("Refreshing pool {}", refreshed_row.pool_address))?;
            refreshed += 1;
        }

        Ok(refreshed)
    }

    async fn tick(&self) -> eyre::Result<()> {
        let existing_rows = scoring_universe(&self.pool, storage::types::venue::DLMM)
            .await
            .wrap_err_with(|| "Loading the known pool universe")?;
        let existing: HashMap<String, PoolForScoring> = existing_rows
            .into_iter()
            .map(|r| (r.pool_address.clone(), r))
            .collect();

        let discovered = self
            .source
            .discover_pools()
            .await
            .wrap_err_with(|| "Discovering pools")?;
        let discovered_set: HashSet<String> =
            discovered.iter().map(|p| p.address.to_string()).collect();

        let missing = existing
            .keys()
            .filter(|addr| !discovered_set.contains(*addr))
            .count();
        if missing > 0 {
            // Detected, not repaired: an address in the database but absent from a fresh gPA
            // scan could mean the pool closed, or a transient RPC gap, or (if this ever
            // happens at scale) a filter regression. None of those should be resolved by
            // silently dropping the row.
            tracing::warn!(
                count = missing,
                "Pools previously onboarded are no longer visible in the chain scan"
            );
        }

        let new_addresses: Vec<_> = discovered
            .iter()
            .filter(|p| !existing.contains_key(&p.address.to_string()))
            .take(self.batch_size)
            .map(|p| p.address)
            .collect();

        let onboarded = self.onboard_new_pools(new_addresses).await?;
        let refreshed = self.refresh_known_pools(&existing).await?;

        tracing::info!(
            discovered = discovered.len(),
            known = existing.len(),
            onboarded,
            refreshed,
            missing,
            "Discovery sweep complete"
        );

        Ok(())
    }
}

#[async_trait]
impl Worker for DiscoveryWorker {
    fn name(&self) -> &'static str {
        "discovery"
    }

    async fn run(&self, ct: CancellationToken) -> eyre::Result<()> {
        tick_loop(ct, self.interval, || self.tick()).await;
        Ok(())
    }
}
