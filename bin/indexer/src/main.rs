use eyre::WrapErr;
use tokio_util::sync::CancellationToken;

use indexer::config::Args;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let config_path = common::config_flag(std::env::args().skip(1));
    let args: Args = common::load_config_with_env(std::env::args_os(), config_path.as_deref())
        .wrap_err_with(|| "Loading configuration")?;
    args.logging.init()?;

    let ct = CancellationToken::new();
    tokio::spawn({
        let ct = ct.clone();
        async move { common::shutdown_signal(ct).await }
    });

    let mut tasks = tokio::task::JoinSet::new();
    tasks.spawn({
        let ct = ct.clone();
        async move { indexer::run(args, ct).await }
    });

    if let Some(first) = tasks.join_next().await {
        let result = match first {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e).wrap_err_with(|| "Indexer task failed unexpectedly"),
            Err(e) => Err(e).wrap_err_with(|| "Indexer task panicked"),
        };
        ct.cancel();
        return result;
    }

    Ok(())
}
