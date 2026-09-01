use clap::Parser;

use crawler::Args;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let args = Args::parse();
    args.logging.init()?;

    crawler::run(args).await
}
