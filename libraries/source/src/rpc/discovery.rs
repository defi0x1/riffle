use eyre::WrapErr;
use solana_account_decoder::{UiAccountEncoding, UiDataSliceConfig};
use solana_client::{
    nonblocking::rpc_client::RpcClient,
    rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig},
    rpc_filter::{Memcmp, RpcFilterType},
};

use crate::PoolMeta;

// Zero-length data slice: discovery becomes a list of pubkeys instead of a list of full
// account bodies, which is the difference between gPA being an occasional cheap scan and
// the thing that gets a provider to rate-limit us. Actual pool state comes from the
// batched reads in `state.rs`, never from here.
pub async fn discover_pools(client: &RpcClient) -> eyre::Result<Vec<PoolMeta>> {
    let config = RpcProgramAccountsConfig {
        filters: Some(vec![RpcFilterType::Memcmp(Memcmp::new_base58_encoded(
            0,
            dlmm_decode::LB_PAIR_DISCRIMINATOR.as_slice(),
        ))]),
        account_config: RpcAccountInfoConfig {
            encoding: Some(UiAccountEncoding::Base64),
            data_slice: Some(UiDataSliceConfig {
                offset: 0,
                length: 0,
            }),
            ..Default::default()
        },
        with_context: Some(false),
        sort_results: Some(false),
    };

    let accounts = client
        .get_program_accounts_with_config(&lb_clmm::ID, config)
        .await
        .wrap_err_with(|| "Discovering pools via getProgramAccounts")?;

    let slot = client
        .get_slot()
        .await
        .wrap_err_with(|| "Getting slot for pool discovery")?;

    Ok(accounts
        .into_iter()
        .map(|(address, _)| PoolMeta {
            address,
            discovered_at_slot: slot,
        })
        .collect())
}
