use std::path::PathBuf;

use chrono::{DateTime, Utc};
use clap::Parser;
use eyre::WrapErr;
use solana_sdk::pubkey::Pubkey;

use crate::pacing::PacingConfig;
use crate::range::RangeSpec;

#[derive(Parser, Debug, Clone)]
#[group(id = "crawler-rpc")]
pub struct RpcConfig {
    /// Solana RPC endpoint the backfill reads from.
    #[arg(long, env)]
    pub rpc_url: String,
}

fn parse_rfc3339(s: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| format!("{s} is not an RFC 3339 timestamp: {e}"))
}

#[derive(Parser, Debug, Clone)]
#[group(id = "crawler-range")]
pub struct RangeConfig {
    /// Pool addresses to backfill, comma-separated. Combined with `--pools-file` if both
    /// are given.
    #[arg(long, env, value_delimiter = ',')]
    pub pools: Vec<String>,

    /// Newline-delimited file of pool addresses. Blank lines and lines starting with `#`
    /// are ignored, so a screening query's output can be piped through `tee` into this file
    /// without editing.
    #[arg(long, env)]
    pub pools_file: Option<PathBuf>,

    /// Lower slot bound (inclusive). Omit to walk as far back as the node retains history
    /// for the pool.
    #[arg(long, env)]
    pub from_slot: Option<u64>,

    /// Upper slot bound (inclusive). Omit to start from the current chain head.
    #[arg(long, env)]
    pub to_slot: Option<u64>,

    /// Lower time bound, RFC 3339 (e.g. `2024-01-01T00:00:00Z`).
    #[arg(long, env, value_parser = parse_rfc3339)]
    pub from_time: Option<DateTime<Utc>>,

    /// Upper time bound, RFC 3339.
    #[arg(long, env, value_parser = parse_rfc3339)]
    pub to_time: Option<DateTime<Utc>>,

    /// Signatures requested per `getSignaturesForAddress` page. The RPC method itself caps
    /// this at 1000.
    #[arg(long, env, default_value_t = 1000)]
    pub page_size: usize,

    /// Buffered rows written per batch, independent of page size -- a page can straddle
    /// several batches on a busy pool, or several pages can share one on a quiet one.
    #[arg(long, env, default_value_t = 200)]
    pub write_batch_size: usize,
}

impl RangeConfig {
    /// Reads and parses `--pools` plus `--pools-file` into a deduplicated address list,
    /// preserving first-seen order so `--pools` entries always sort before file entries.
    pub fn resolve_pools(&self) -> eyre::Result<Vec<Pubkey>> {
        let mut seen = std::collections::HashSet::new();
        let mut pools = Vec::new();

        for raw in &self.pools {
            push_pool(&mut pools, &mut seen, raw)?;
        }

        if let Some(path) = &self.pools_file {
            let contents = std::fs::read_to_string(path)
                .wrap_err_with(|| format!("Reading pools file {}", path.display()))?;
            for line in contents.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                push_pool(&mut pools, &mut seen, trimmed)?;
            }
        }

        if pools.is_empty() {
            eyre::bail!("No pools given: pass --pools and/or --pools-file");
        }

        Ok(pools)
    }

    pub fn spec(&self) -> RangeSpec {
        RangeSpec {
            from_slot: self.from_slot,
            to_slot: self.to_slot,
            from_time: self.from_time.map(|t| t.timestamp()),
            to_time: self.to_time.map(|t| t.timestamp()),
        }
    }
}

fn push_pool(
    pools: &mut Vec<Pubkey>,
    seen: &mut std::collections::HashSet<Pubkey>,
    raw: &str,
) -> eyre::Result<()> {
    let pubkey: Pubkey = raw
        .parse()
        .wrap_err_with(|| format!("Parsing pool address {raw}"))?;
    if seen.insert(pubkey) {
        pools.push(pubkey);
    }
    Ok(())
}

#[derive(Parser, Debug, Clone)]
pub struct Args {
    #[clap(flatten)]
    pub logging: logger::Config,

    #[clap(flatten)]
    pub postgres: common::PostgresConfig,

    #[clap(flatten)]
    pub rpc: RpcConfig,

    #[clap(flatten)]
    pub pacing: PacingConfig,

    #[clap(flatten)]
    pub range: RangeConfig,

    /// Where progress is recorded so an interrupted run can resume near where it stopped.
    #[arg(long, env, default_value = "crawler_checkpoint.json")]
    pub checkpoint_file: PathBuf,

    /// Walk the range and report what would be fetched and written, without touching
    /// Postgres or the checkpoint file.
    #[arg(long, env, default_value_t = false)]
    pub dry_run: bool,
}

// rpc_url and database_url may carry embedded credentials, so logging goes through this impl
// rather than the derived Debug.
impl std::fmt::Display for Args {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "crawler::Args {{ log_level: {}, log_format: {:?}, postgres: {}, rpc_url: <redacted>, \
             max_concurrent_rpc: {}, min_request_interval: {:?}, max_retries: {}, \
             backoff_base: {:?}, backoff_max: {:?}, pools: {}, pools_file: {:?}, \
             from_slot: {:?}, to_slot: {:?}, from_time: {:?}, to_time: {:?}, \
             page_size: {}, write_batch_size: {}, checkpoint_file: {:?}, dry_run: {} }}",
            self.logging.log_level,
            self.logging.log_format,
            self.postgres,
            self.pacing.max_concurrent_rpc,
            self.pacing.min_request_interval,
            self.pacing.max_retries,
            self.pacing.backoff_base,
            self.pacing.backoff_max,
            self.range.pools.len(),
            self.range.pools_file,
            self.range.from_slot,
            self.range.to_slot,
            self.range.from_time,
            self.range.to_time,
            self.range.page_size,
            self.range.write_batch_size,
            self.checkpoint_file,
            self.dry_run,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_pools_merges_and_dedupes_flag_and_file() {
        let dir = std::env::temp_dir().join(format!("crawler-cli-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("pools.txt");
        let a = Pubkey::new_unique();
        let b = Pubkey::new_unique();
        std::fs::write(&file, format!("# comment\n\n{b}\n{a}\n")).unwrap();

        let cfg = RangeConfig {
            pools: vec![a.to_string()],
            pools_file: Some(file),
            from_slot: None,
            to_slot: None,
            from_time: None,
            to_time: None,
            page_size: 1000,
            write_batch_size: 200,
        };

        let resolved = cfg.resolve_pools().unwrap();
        assert_eq!(resolved, vec![a, b]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_resolve_pools_rejects_empty_input() {
        let cfg = RangeConfig {
            pools: vec![],
            pools_file: None,
            from_slot: None,
            to_slot: None,
            from_time: None,
            to_time: None,
            page_size: 1000,
            write_batch_size: 200,
        };
        assert!(cfg.resolve_pools().is_err());
    }

    #[test]
    fn test_resolve_pools_rejects_malformed_address() {
        let cfg = RangeConfig {
            pools: vec!["not-a-pubkey".to_string()],
            pools_file: None,
            from_slot: None,
            to_slot: None,
            from_time: None,
            to_time: None,
            page_size: 1000,
            write_batch_size: 200,
        };
        assert!(cfg.resolve_pools().is_err());
    }

    #[test]
    fn test_spec_converts_rfc3339_times_to_unix_seconds() {
        let cfg = RangeConfig {
            pools: vec![],
            pools_file: None,
            from_slot: Some(5),
            to_slot: None,
            from_time: Some(parse_rfc3339("2024-01-01T00:00:00Z").unwrap()),
            to_time: None,
            page_size: 1000,
            write_batch_size: 200,
        };
        let spec = cfg.spec();
        assert_eq!(spec.from_slot, Some(5));
        assert_eq!(spec.from_time, Some(1_704_067_200));
    }
}
