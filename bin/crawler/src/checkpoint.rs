//! Resumability. The crawler's own state, kept next to it as a small JSON file rather than a
//! new database table -- correctness never depends on this file (every write path is an
//! upsert or an `ON CONFLICT DO NOTHING` insert keyed on real chain identity, so replaying a
//! range twice is always safe), it only saves an interrupted run from re-walking and
//! re-fetching transactions it already paid for.

use std::collections::HashMap;
use std::path::Path;

use eyre::WrapErr;
use serde::{Deserialize, Serialize};

use crate::range::RangeSpec;

/// Progress for one pool's backward walk. `cursor` is the oldest signature reached so far --
/// the next page resumes with it as `before` rather than starting back at the chain head.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PoolCheckpoint {
    pub from_slot: Option<u64>,
    pub to_slot: Option<u64>,
    pub from_time: Option<i64>,
    pub to_time: Option<i64>,
    pub cursor: Option<String>,
    pub complete: bool,
    pub transactions_seen: u64,
    pub rows_written: u64,
}

impl PoolCheckpoint {
    fn matches_range(&self, spec: &RangeSpec) -> bool {
        self.from_slot == spec.from_slot
            && self.to_slot == spec.to_slot
            && self.from_time == spec.from_time
            && self.to_time == spec.to_time
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Checkpoints {
    pub pools: HashMap<String, PoolCheckpoint>,
}

impl Checkpoints {
    /// A missing file is the fresh-start case, not an error -- the first run against a
    /// checkpoint path always looks like this.
    pub fn load(path: &Path) -> eyre::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)
            .wrap_err_with(|| format!("Reading checkpoint file {}", path.display()))?;
        serde_json::from_str(&raw)
            .wrap_err_with(|| format!("Parsing checkpoint file {}", path.display()))
    }

    /// Write-to-temp-then-rename so a crawler killed mid-save never leaves a half-written,
    /// unparseable checkpoint file behind for the next run to trip over.
    pub fn save(&self, path: &Path) -> eyre::Result<()> {
        let body =
            serde_json::to_string_pretty(self).wrap_err_with(|| "Serialising checkpoint state")?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, body)
            .wrap_err_with(|| format!("Writing checkpoint temp file {}", tmp.display()))?;
        std::fs::rename(&tmp, path)
            .wrap_err_with(|| format!("Replacing checkpoint file {}", path.display()))?;
        Ok(())
    }
}

/// What a pool's walk should do given its stored checkpoint (if any) and the range this run
/// was asked to cover.
#[derive(Clone, Debug, PartialEq)]
pub enum ResumePlan {
    /// No usable checkpoint: page from the chain head.
    Fresh,
    /// Continue paging backward from this signature.
    Resume { before: String },
    /// The checkpoint already covers this exact range end to end.
    AlreadyComplete,
}

/// Pure decision: a checkpoint only resumes a walk over the *same* range it was recorded
/// against. An operator who reruns with a wider or shifted range gets a fresh walk rather
/// than a silently truncated one -- resuming a mismatched checkpoint would look like a
/// correct backfill while quietly skipping the newly requested part of the range.
pub fn resume_plan(existing: Option<&PoolCheckpoint>, spec: &RangeSpec) -> ResumePlan {
    let Some(checkpoint) = existing else {
        return ResumePlan::Fresh;
    };
    if !checkpoint.matches_range(spec) {
        return ResumePlan::Fresh;
    }
    if checkpoint.complete {
        return ResumePlan::AlreadyComplete;
    }
    match &checkpoint.cursor {
        Some(sig) => ResumePlan::Resume {
            before: sig.clone(),
        },
        None => ResumePlan::Fresh,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> RangeSpec {
        RangeSpec {
            from_slot: Some(10),
            to_slot: Some(100),
            from_time: None,
            to_time: None,
        }
    }

    #[test]
    fn test_no_checkpoint_is_fresh() {
        assert_eq!(resume_plan(None, &spec()), ResumePlan::Fresh);
    }

    #[test]
    fn test_incomplete_checkpoint_resumes_from_cursor() {
        let cp = PoolCheckpoint {
            from_slot: Some(10),
            to_slot: Some(100),
            from_time: None,
            to_time: None,
            cursor: Some("sig123".to_string()),
            complete: false,
            transactions_seen: 5,
            rows_written: 3,
        };
        assert_eq!(
            resume_plan(Some(&cp), &spec()),
            ResumePlan::Resume {
                before: "sig123".to_string()
            }
        );
    }

    #[test]
    fn test_complete_checkpoint_over_same_range_is_skipped() {
        let cp = PoolCheckpoint {
            from_slot: Some(10),
            to_slot: Some(100),
            from_time: None,
            to_time: None,
            cursor: Some("sig123".to_string()),
            complete: true,
            transactions_seen: 5,
            rows_written: 3,
        };
        assert_eq!(resume_plan(Some(&cp), &spec()), ResumePlan::AlreadyComplete);
    }

    #[test]
    fn test_checkpoint_over_a_different_range_is_ignored() {
        let cp = PoolCheckpoint {
            from_slot: Some(0),
            to_slot: Some(50),
            from_time: None,
            to_time: None,
            cursor: Some("sig123".to_string()),
            complete: true,
            transactions_seen: 5,
            rows_written: 3,
        };
        assert_eq!(resume_plan(Some(&cp), &spec()), ResumePlan::Fresh);
    }

    #[test]
    fn test_checkpoint_with_no_cursor_yet_is_fresh() {
        let cp = PoolCheckpoint {
            from_slot: Some(10),
            to_slot: Some(100),
            from_time: None,
            to_time: None,
            cursor: None,
            complete: false,
            transactions_seen: 0,
            rows_written: 0,
        };
        assert_eq!(resume_plan(Some(&cp), &spec()), ResumePlan::Fresh);
    }

    #[test]
    fn test_save_then_load_round_trips() {
        let dir = std::env::temp_dir().join(format!("crawler-ckpt-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("checkpoint.json");

        let mut checkpoints = Checkpoints::default();
        checkpoints.pools.insert(
            "pool1".to_string(),
            PoolCheckpoint {
                from_slot: Some(1),
                to_slot: None,
                from_time: None,
                to_time: None,
                cursor: Some("sigABC".to_string()),
                complete: false,
                transactions_seen: 42,
                rows_written: 7,
            },
        );
        checkpoints.save(&path).unwrap();

        let loaded = Checkpoints::load(&path).unwrap();
        assert_eq!(loaded.pools.get("pool1"), checkpoints.pools.get("pool1"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_load_missing_file_is_empty_not_an_error() {
        let path = std::env::temp_dir().join("crawler-ckpt-does-not-exist.json");
        std::fs::remove_file(&path).ok();
        let loaded = Checkpoints::load(&path).unwrap();
        assert!(loaded.pools.is_empty());
    }
}
