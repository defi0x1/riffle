use std::collections::HashSet;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use eyre::WrapErr;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;

use common::{Worker, tick_loop};
use storage::queries::{open_paper_positions, top_pools, watch_set};
use storage::types::{Timeframe, venue};
use storage::write::{demote_pools, never_measured_pools, promote_pools};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TierChanges {
    pub promote: Vec<String>,
    pub demote: Vec<String>,
}

/// The pure promotion/demotion decision, kept free of I/O so it can be tested directly.
///
/// - `ranked`: the top `rank_slots` pools by measured/screened `r_org`, descending.
/// - `unmeasured`: pools with no indicator row yet, in some stable order; the first
///   `exploration_n` of these are reserved regardless of rank, so a weak screening prior can
///   never permanently hide a pool that has never actually been looked at.
/// - `watched`: the current tier-1 set.
/// - `safe_ranked`: a wider ranked list than `ranked` (rank cutoff plus a hysteresis margin)
///   -- a watched pool that still clears this wider bar is not demoted, so a pool sitting
///   near the cutoff does not flap in and out on every sweep.
/// - `open_positions`: pools with an open paper position. Demotion is skipped for these here
///   as a first line of defence; `demote_pools` enforces the same rule again at the storage
///   layer, which is the actual source of truth today (see the worker below).
pub fn select_tier_changes(
    ranked: &[String],
    unmeasured: &[String],
    watched: &[String],
    safe_ranked: &[String],
    exploration_n: usize,
    open_positions: &HashSet<String>,
) -> TierChanges {
    let mut promote: Vec<String> = ranked.to_vec();
    for addr in unmeasured.iter().take(exploration_n) {
        if !promote.contains(addr) {
            promote.push(addr.clone());
        }
    }

    let mut safe: HashSet<String> = safe_ranked.iter().cloned().collect();
    // A never-measured pool is protected until it has actually had a chance to be scored --
    // otherwise a pool promoted this tick via the exploration slice would be demoted right
    // back out on the very same sweep, before scorer ever sees it.
    safe.extend(unmeasured.iter().cloned());
    safe.extend(open_positions.iter().cloned());

    let demote: Vec<String> = watched
        .iter()
        .filter(|addr| !safe.contains(*addr))
        .cloned()
        .collect();

    TierChanges { promote, demote }
}

/// Re-evaluates tier membership on a fixed interval against the screening rank already
/// computed by the scorer (`indicators_10m.r_org`). This worker only selects and applies the
/// change; it computes no ranking of its own.
pub struct TierWorker {
    pool: PgPool,
    interval: Duration,
    max_watched: i64,
    exploration_slice: f64,
    demotion_margin: i64,
}

impl TierWorker {
    pub fn new(
        pool: PgPool,
        interval: Duration,
        max_watched: i64,
        exploration_slice: f64,
        demotion_margin: i64,
    ) -> Self {
        Self {
            pool,
            interval,
            max_watched,
            exploration_slice,
            demotion_margin,
        }
    }

    async fn tick(&self) -> eyre::Result<()> {
        let cap = self.max_watched.max(0);
        let exploration_n = ((cap as f64) * self.exploration_slice.clamp(0.0, 1.0)).round() as i64;
        let exploration_n = exploration_n.clamp(0, cap);
        let rank_slots = cap - exploration_n;

        let ranked = top_pools(&self.pool, venue::DLMM, Timeframe::M10, rank_slots)
            .await
            .wrap_err_with(|| "Loading ranked screening candidates")?;
        let safe_ranked = top_pools(
            &self.pool,
            venue::DLMM,
            Timeframe::M10,
            rank_slots + self.demotion_margin.max(0),
        )
        .await
        .wrap_err_with(|| "Loading the demotion-hysteresis candidate set")?;
        let unmeasured = never_measured_pools(&self.pool, cap.max(exploration_n))
            .await
            .wrap_err_with(|| "Loading never-measured pools")?;
        let watched = watch_set(&self.pool)
            .await
            .wrap_err_with(|| "Loading the current watch set")?;
        let open_positions = open_paper_positions(&self.pool)
            .await
            .wrap_err_with(|| "Loading open paper positions")?;

        let ranked_addrs: Vec<String> = ranked.into_iter().map(|r| r.pool_address).collect();
        let safe_ranked_addrs: Vec<String> =
            safe_ranked.into_iter().map(|r| r.pool_address).collect();
        let watched_addrs: Vec<String> = watched.into_iter().map(|w| w.pool_address).collect();
        let open_set: HashSet<String> =
            open_positions.into_iter().map(|p| p.pool_address).collect();

        let changes = select_tier_changes(
            &ranked_addrs,
            &unmeasured,
            &watched_addrs,
            &safe_ranked_addrs,
            exploration_n.max(0) as usize,
            &open_set,
        );

        let now = Utc::now();
        let promoted = promote_pools(&self.pool, &changes.promote, now).await?;
        let demoted = demote_pools(&self.pool, &changes.demote, now).await?;

        tracing::info!(
            promoted,
            demoted = demoted.len(),
            watched = watched_addrs.len(),
            cap,
            "Tier sweep complete"
        );

        Ok(())
    }
}

#[async_trait]
impl Worker for TierWorker {
    fn name(&self) -> &'static str {
        "tier"
    }

    async fn run(&self, ct: CancellationToken) -> eyre::Result<()> {
        tick_loop(ct, self.interval, || self.tick()).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addrs(prefix: &str, n: usize) -> Vec<String> {
        (0..n).map(|i| format!("{prefix}{i}")).collect()
    }

    #[test]
    fn test_promotion_combines_ranked_and_exploration_slice() {
        let ranked = addrs("ranked", 3);
        let unmeasured = addrs("new", 5);
        let changes = select_tier_changes(&ranked, &unmeasured, &[], &ranked, 2, &HashSet::new());

        assert_eq!(changes.promote.len(), 5);
        assert!(changes.promote.contains(&"new0".to_string()));
        assert!(changes.promote.contains(&"new1".to_string()));
        assert!(!changes.promote.contains(&"new2".to_string()));
    }

    #[test]
    fn test_exploration_slice_does_not_duplicate_an_already_ranked_pool() {
        let ranked = vec!["a".to_string(), "b".to_string()];
        let unmeasured = vec!["a".to_string(), "c".to_string()];
        let changes = select_tier_changes(&ranked, &unmeasured, &[], &ranked, 2, &HashSet::new());

        assert_eq!(changes.promote, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_watched_pool_outside_the_safe_set_is_demoted() {
        let watched = vec!["stale".to_string()];
        let safe_ranked = vec!["other".to_string()];
        let changes = select_tier_changes(&[], &[], &watched, &safe_ranked, 0, &HashSet::new());

        assert_eq!(changes.demote, vec!["stale"]);
    }

    #[test]
    fn test_hysteresis_protects_a_pool_still_inside_the_wider_bar() {
        // "borderline" would be cut by the strict rank list but still clears the wider,
        // margin-padded safe list -- it must not be demoted.
        let ranked = vec!["top".to_string()];
        let safe_ranked = vec!["top".to_string(), "borderline".to_string()];
        let watched = vec!["top".to_string(), "borderline".to_string()];
        let changes = select_tier_changes(&ranked, &[], &watched, &safe_ranked, 0, &HashSet::new());

        assert!(changes.demote.is_empty());
    }

    #[test]
    fn test_open_position_pool_is_never_demoted_even_if_unranked() {
        let watched = vec!["has_position".to_string(), "no_position".to_string()];
        let safe_ranked: Vec<String> = Vec::new();
        let mut open = HashSet::new();
        open.insert("has_position".to_string());

        let changes = select_tier_changes(&[], &[], &watched, &safe_ranked, 0, &open);

        assert_eq!(changes.demote, vec!["no_position"]);
    }

    #[test]
    fn test_never_measured_watched_pool_survives_its_own_promotion_tick() {
        // A pool promoted this tick via the exploration slice has, by definition, no
        // indicators yet -- it must not be demoted on the very same sweep.
        let unmeasured = vec!["fresh".to_string()];
        let watched = vec!["fresh".to_string()];
        let changes = select_tier_changes(&[], &unmeasured, &watched, &[], 1, &HashSet::new());

        assert!(changes.promote.contains(&"fresh".to_string()));
        assert!(changes.demote.is_empty());
    }
}
