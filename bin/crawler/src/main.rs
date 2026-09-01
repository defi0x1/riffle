use eyre::WrapErr;

use crawler::Args;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let config_path = common::config_flag(std::env::args().skip(1));
    let args: Args = common::load_config_with_env(std::env::args_os(), config_path.as_deref())
        .wrap_err_with(|| "Loading configuration")?;
    args.logging.init()?;

    crawler::run(args).await
}
