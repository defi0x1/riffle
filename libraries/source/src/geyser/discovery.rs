use std::collections::HashMap;
use std::time::Duration;

use futures::StreamExt;
use solana_sdk::pubkey::Pubkey;
use yellowstone_grpc_proto::prelude::subscribe_update::UpdateOneof;

use crate::{GeyserConfig, PoolMeta};

use super::connection::ConnectionConfig;
use super::filters;

// How long to wait after the last new pool before assuming the initial snapshot burst is
// done, plus a hard ceiling in case the burst never quiesces (a very active universe, or a
// provider that streams live updates immediately rather than a burst).
const IDLE_TIMEOUT: Duration = Duration::from_secs(3);
const HARD_CAP: Duration = Duration::from_secs(20);

pub async fn discover_pools(config: &GeyserConfig) -> eyre::Result<Vec<PoolMeta>> {
    let conn_cfg = ConnectionConfig::new(config)?;
    let commitment = filters::parse_commitment(&config.geyser_commitment)?;

    let mut client = super::connection::connect(&conn_cfg).await?;
    let request = filters::discovery_subscribe_request(commitment);
    let (_sink, mut stream) = client
        .subscribe_with_request(Some(request))
        .await
        .map_err(|e| eyre::eyre!("Subscribing for pool discovery: {e}"))?;

    let mut pools: HashMap<Pubkey, u64> = HashMap::new();

    let idle = tokio::time::sleep(IDLE_TIMEOUT);
    tokio::pin!(idle);
    let hard_cap = tokio::time::sleep(HARD_CAP);
    tokio::pin!(hard_cap);

    loop {
        tokio::select! {
            () = &mut idle => break,
            () = &mut hard_cap => break,
            item = stream.next() => {
                match item {
                    Some(Ok(update)) => {
                        if let Some(UpdateOneof::Account(acc)) = &update.update_oneof
                            && let Some(info) = &acc.account
                            && dlmm_decode::decode_lb_pair(&info.data).is_ok()
                            && let Ok(pubkey) = Pubkey::try_from(info.pubkey.as_slice())
                        {
                            pools.insert(pubkey, acc.slot);
                            idle.as_mut().reset(tokio::time::Instant::now() + IDLE_TIMEOUT);
                        }
                    }
                    Some(Err(status)) => {
                        tracing::warn!(error = ?status, "Pool discovery stream error, using what was collected so far");
                        break;
                    }
                    None => break,
                }
            }
        }
    }

    Ok(pools
        .into_iter()
        .map(|(address, discovered_at_slot)| PoolMeta {
            address,
            discovered_at_slot,
        })
        .collect())
}
