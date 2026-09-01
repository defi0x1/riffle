use std::sync::Arc;

use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use sqlx::PgPool;

use crate::config::Args;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub rpc: Arc<RpcClient>,
    pub config: Arc<Args>,
}
