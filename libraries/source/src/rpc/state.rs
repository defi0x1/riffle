use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use backon::{ExponentialBuilder, Retryable};
use eyre::WrapErr;
use futures::stream::{self, BoxStream, StreamExt};
use solana_account_decoder::UiAccountEncoding;
use solana_client::{nonblocking::rpc_client::RpcClient, rpc_config::RpcAccountInfoConfig};
use solana_sdk::{account::Account, pubkey::Pubkey};
use tokio::sync::{Mutex, Semaphore};

use dlmm_decode::{decode_bin_array, decode_lb_pair};
use metrics::{RPC_CALL_DURATION_SECS, RPC_CALL_TOTAL};

use crate::{StateUpdate, WatchSet};

use super::batching::{AccountGroup, MAX_POOL_KEYS_PER_BATCH, pack_groups};
use super::clock::{CLOCK_SYSVAR, decode_clock};

fn bin_array_pda(lb_pair: &Pubkey, index: i64) -> Pubkey {
    Pubkey::find_program_address(
        &[
            lb_clmm::utils::seeds::BIN_ARRAY,
            lb_pair.as_ref(),
            &index.to_le_bytes(),
        ],
        &lb_clmm::ID,
    )
    .0
}

fn account_group_for(pool: Pubkey, last_active_bin_id: Option<i32>) -> AccountGroup {
    let Some(active_bin_id) = last_active_bin_id else {
        return AccountGroup::lb_pair_only(pool);
    };

    // Only fails on i32 overflow at the extreme edge of the bin id range, which real pools
    // never reach; falling back to array 0 there is harmless since the next tick corrects it.
    let center =
        lb_clmm::state::bin::BinArray::bin_id_to_bin_array_index(active_bin_id).unwrap_or(0) as i64;
    let bin_arrays = [
        bin_array_pda(&pool, center - 1),
        bin_array_pda(&pool, center),
        bin_array_pda(&pool, center + 1),
    ];
    AccountGroup::with_bin_arrays(pool, bin_arrays)
}

pub struct StatePoller {
    client: Arc<RpcClient>,
    semaphore: Arc<Semaphore>,
    max_retries: usize,
}

impl StatePoller {
    pub fn new(client: Arc<RpcClient>, max_concurrent: usize, max_retries: usize) -> Self {
        Self {
            client,
            semaphore: Arc::new(Semaphore::new(max_concurrent.max(1))),
            max_retries,
        }
    }

    async fn get_multiple_accounts(
        &self,
        keys: &[Pubkey],
    ) -> eyre::Result<(u64, Vec<Option<Account>>)> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .wrap_err_with(|| "Acquiring RPC concurrency permit")?;

        let client = self.client.clone();
        let keys = keys.to_vec();
        let max_retries = self.max_retries;
        let started = std::time::Instant::now();

        let outcome = (|| {
            let client = client.clone();
            let keys = keys.clone();
            async move {
                client
                    .get_multiple_accounts_with_config(
                        &keys,
                        RpcAccountInfoConfig {
                            encoding: Some(UiAccountEncoding::Base64),
                            ..Default::default()
                        },
                    )
                    .await
            }
        })
        .retry(
            ExponentialBuilder::default()
                .with_jitter()
                .with_max_times(max_retries),
        )
        .notify(
            |err: &solana_client::client_error::ClientError, delay: Duration| {
                tracing::warn!(error = ?err, delay = ?delay, "Retrying getMultipleAccounts");
            },
        )
        .await;

        RPC_CALL_DURATION_SECS
            .with_label_values(&["getMultipleAccounts"])
            .observe(started.elapsed().as_secs_f64());

        let response = match outcome {
            Ok(r) => {
                RPC_CALL_TOTAL
                    .with_label_values(&["getMultipleAccounts", "ok"])
                    .inc();
                r
            }
            Err(e) => {
                RPC_CALL_TOTAL
                    .with_label_values(&["getMultipleAccounts", "error"])
                    .inc();
                return Err(e).wrap_err_with(|| "Fetching account batch");
            }
        };

        Ok((response.context.slot, response.value))
    }

    async fn fetch_state_batch(&self, groups: &[AccountGroup]) -> eyre::Result<Vec<StateUpdate>> {
        let mut keys: Vec<Pubkey> = groups.iter().flat_map(|g| g.keys.iter().copied()).collect();
        keys.push(CLOCK_SYSVAR);

        let (slot, mut values) = self.get_multiple_accounts(&keys).await?;

        let clock_account = values
            .pop()
            .flatten()
            .ok_or_else(|| eyre::eyre!("Clock sysvar account missing from batch response"))?;
        let clock = decode_clock(&clock_account)?;

        let mut updates = Vec::with_capacity(groups.len());
        let mut cursor = 0usize;
        for group in groups {
            let group_values = &values[cursor..cursor + group.len()];
            cursor += group.len();

            let lb_pair = match group_values.first().and_then(|v| v.as_ref()) {
                Some(account) => match decode_lb_pair(&account.data) {
                    Ok(state) => Some(state),
                    Err(e) => {
                        tracing::error!(error = ?e, pool = %group.pool, "Failed to decode LbPair");
                        None
                    }
                },
                None => None,
            };

            let bin_arrays = group_values[1..]
                .iter()
                .filter_map(|v| v.as_ref())
                .filter_map(|account| match decode_bin_array(&account.data) {
                    Ok(state) => Some(state),
                    Err(e) => {
                        tracing::error!(error = ?e, pool = %group.pool, "Failed to decode BinArray");
                        None
                    }
                })
                .collect();

            updates.push(StateUpdate {
                pool: group.pool,
                slot,
                block_time: clock.unix_timestamp,
                lb_pair,
                bin_arrays,
            });
        }

        Ok(updates)
    }

    pub async fn poll_all(
        &self,
        pools: &[Pubkey],
        last_active_bin: &HashMap<Pubkey, i32>,
    ) -> eyre::Result<Vec<StateUpdate>> {
        let groups: Vec<AccountGroup> = pools
            .iter()
            .map(|p| account_group_for(*p, last_active_bin.get(p).copied()))
            .collect();

        let batches = pack_groups(groups, MAX_POOL_KEYS_PER_BATCH)?;

        let mut all_updates = Vec::new();
        for batch in batches {
            match self.fetch_state_batch(&batch).await {
                Ok(updates) => all_updates.extend(updates),
                Err(e) => {
                    tracing::error!(error = ?e, "Failed to fetch a state batch, skipping it this tick")
                }
            }
        }
        Ok(all_updates)
    }
}

struct StreamState {
    pools: Vec<Pubkey>,
    pending: VecDeque<StateUpdate>,
    started: bool,
}

pub fn state_stream(
    poller: Arc<StatePoller>,
    watched: WatchSet,
    interval: Duration,
) -> BoxStream<'static, StateUpdate> {
    let last_active_bin: Arc<Mutex<HashMap<Pubkey, i32>>> = Arc::new(Mutex::new(HashMap::new()));

    stream::unfold(
        StreamState {
            pools: watched.pools,
            pending: VecDeque::new(),
            started: false,
        },
        move |mut state| {
            let poller = poller.clone();
            let last_active_bin = last_active_bin.clone();
            async move {
                loop {
                    if let Some(update) = state.pending.pop_front() {
                        return Some((update, state));
                    }

                    if state.started {
                        tokio::time::sleep(interval).await;
                    }
                    state.started = true;

                    let snapshot = last_active_bin.lock().await.clone();
                    match poller.poll_all(&state.pools, &snapshot).await {
                        Ok(updates) => {
                            if !updates.is_empty() {
                                let mut guard = last_active_bin.lock().await;
                                for update in &updates {
                                    if let Some(lb_pair) = &update.lb_pair {
                                        guard.insert(update.pool, lb_pair.active_bin_id);
                                    }
                                }
                            }
                            state.pending.extend(updates);
                        }
                        Err(e) => tracing::error!(error = ?e, "State poll tick failed"),
                    }
                }
            }
        },
    )
    .boxed()
}
