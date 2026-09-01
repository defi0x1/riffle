use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

#[async_trait]
pub trait Worker: Send + Sync {
    fn name(&self) -> &'static str;

    async fn run(&self, ct: CancellationToken) -> eyre::Result<()>;
}
