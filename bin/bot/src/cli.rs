// Commands are parsed with clap over the raw message text rather than hand-rolled string
// matching: splitting on whitespace and handing the tokens to `try_parse_from` gives typed
// arguments, subcommands and `--help` for free, and clap's own error text is good enough to
// send straight back to the chat.
use clap::{Parser, Subcommand, ValueEnum};
use rust_decimal::Decimal;

use storage::types::Timeframe;

#[derive(Parser, Debug, Clone, PartialEq)]
#[command(
    name = "",
    no_binary_name = true,
    disable_help_subcommand = true,
    subcommand_required = true,
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug, Clone, PartialEq)]
#[command(rename_all = "lowercase")]
pub enum Command {
    /// Activity ranking -- what is hot right now.
    Top {
        #[arg(default_value = "5m")]
        tf: TimeframeArg,
    },
    /// Highest volume-to-TVL ranking, with change against the previous bucket.
    Volume {
        #[arg(default_value = "5m")]
        tf: TimeframeArg,
    },
    /// Our own fee-over-risk ranking, gate-filtered -- what actually pays.
    Potential {
        #[arg(default_value = "5m")]
        tf: TimeframeArg,
    },
    /// Pool metadata plus every timeframe's indicators.
    Pool { address: String },
    /// Full rationale for a pool, including why it did not qualify.
    Why { address: String },
    /// Forces tier-1 (measured) membership for a pool; pass `off` to release it.
    Watch {
        address: String,
        action: Option<WatchAction>,
    },
    /// Suppresses signals for a pool for a duration, e.g. `2h`, `45m`.
    Mute {
        address: String,
        duration: humantime::Duration,
    },
    /// Ingest lag per source, slot gaps, tier size.
    Status,

    /// Registers your Solana public key, or lists your registered wallets if none is given.
    /// Only ever accepts a public key -- signing happens on your own device in the Mini App.
    Wallet {
        pubkey: Option<String>,
        label: Option<String>,
    },
    /// Latest token balances for a registered wallet (yours, if you have exactly one).
    Balance { wallet: Option<String> },
    /// Open positions for a registered wallet (yours, if you have exactly one).
    Positions { wallet: Option<String> },
    /// Deposits-vs-withdrawals-plus-current-value for one of your positions.
    Profit { position: String },

    /// Proposes opening a new position in a pool, sized by bin width and centered on the
    /// pool's most recently observed active bin -- a width plus the pool address is
    /// everything a row from /potential already gives you, so there is no bin id to copy by
    /// hand. Reviewed and signed in the Mini App; the position's own keypair is generated on
    /// your device there, this chat never sees it.
    Open {
        address: String,
        #[arg(value_parser = clap::value_parser!(u8).range(1..=70))]
        width: u8,
    },
    /// Proposes adding liquidity to one of your positions. Reviewed and signed in the Mini
    /// App -- this chat only ever describes the proposal, it never moves funds itself.
    Add {
        position: String,
        amount_x: Decimal,
        amount_y: Decimal,
    },
    /// Proposes withdrawing a share of one of your positions. Reviewed and signed in the Mini
    /// App.
    Remove {
        position: String,
        #[arg(value_parser = clap::value_parser!(u8).range(1..=100))]
        percent: u8,
    },
    /// Proposes claiming accrued fees on one of your positions. Reviewed and signed in the
    /// Mini App.
    Claim { position: String },
    /// Proposes closing one of your positions entirely. Reviewed and signed in the Mini App.
    Close { position: String },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum WatchAction {
    Off,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum TimeframeArg {
    #[value(name = "5m")]
    M5,
    #[value(name = "10m")]
    M10,
    #[value(name = "1h")]
    H1,
    #[value(name = "4h")]
    H4,
    #[value(name = "24h")]
    H24,
}

impl From<TimeframeArg> for Timeframe {
    fn from(tf: TimeframeArg) -> Self {
        match tf {
            TimeframeArg::M5 => Timeframe::M5,
            TimeframeArg::M10 => Timeframe::M10,
            TimeframeArg::H1 => Timeframe::H1,
            TimeframeArg::H4 => Timeframe::H4,
            TimeframeArg::H24 => Timeframe::H24,
        }
    }
}

// Telegram sends "/top@MyBotName" in groups and always includes the leading slash; clap
// only wants the bare subcommand name. `secret_guard` reuses this to check the command name
// on raw text before any tokenizing that would otherwise happen inside clap itself.
pub(crate) fn normalize_command_token(token: &str) -> String {
    let stripped = token.trim_start_matches('/');
    stripped.split('@').next().unwrap_or(stripped).to_string()
}

pub fn parse_command(text: &str) -> Result<Command, clap::Error> {
    let mut tokens: Vec<String> = text.split_whitespace().map(str::to_string).collect();
    if let Some(first) = tokens.first_mut() {
        *first = normalize_command_token(first);
    }
    Cli::try_parse_from(tokens).map(|cli| cli.command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parses_top_with_default_timeframe() {
        let command = parse_command("/top").unwrap();
        assert_eq!(
            command,
            Command::Top {
                tf: TimeframeArg::M5
            }
        );
    }

    #[test]
    fn test_parses_top_with_explicit_timeframe() {
        let command = parse_command("/top 1h").unwrap();
        assert_eq!(
            command,
            Command::Top {
                tf: TimeframeArg::H1
            }
        );
    }

    #[test]
    fn test_strips_group_mention_suffix() {
        let command = parse_command("/status@FeeFarmBot").unwrap();
        assert_eq!(command, Command::Status);
    }

    #[test]
    fn test_parses_pool_address() {
        let command = parse_command("/pool 7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU").unwrap();
        assert_eq!(
            command,
            Command::Pool {
                address: "7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU".to_string()
            }
        );
    }

    #[test]
    fn test_pool_without_address_is_rejected() {
        assert!(parse_command("/pool").is_err());
    }

    #[test]
    fn test_parses_watch_off() {
        let command = parse_command("/watch addr1 off").unwrap();
        assert_eq!(
            command,
            Command::Watch {
                address: "addr1".to_string(),
                action: Some(WatchAction::Off),
            }
        );
    }

    #[test]
    fn test_parses_watch_without_action() {
        let command = parse_command("/watch addr1").unwrap();
        assert_eq!(
            command,
            Command::Watch {
                address: "addr1".to_string(),
                action: None,
            }
        );
    }

    #[test]
    fn test_watch_rejects_unknown_action() {
        assert!(parse_command("/watch addr1 disable").is_err());
    }

    #[test]
    fn test_parses_mute_duration() {
        let command = parse_command("/mute addr1 2h").unwrap();
        match command {
            Command::Mute { address, duration } => {
                assert_eq!(address, "addr1");
                assert_eq!(*duration, std::time::Duration::from_secs(2 * 3600));
            }
            other => panic!("expected Mute, got {other:?}"),
        }
    }

    #[test]
    fn test_mute_rejects_malformed_duration() {
        let err = parse_command("/mute addr1 notaduration").unwrap_err();
        // Rendered straight back to the chat, so it has to actually say something useful.
        assert!(err.to_string().to_lowercase().contains("duration") || !err.to_string().is_empty());
    }

    #[test]
    fn test_unknown_command_is_rejected() {
        assert!(parse_command("/frobnicate").is_err());
    }

    #[test]
    fn test_top_rejects_unknown_timeframe() {
        assert!(parse_command("/top 3m").is_err());
    }

    #[test]
    fn test_parses_wallet_with_no_args_as_a_list_request() {
        let command = parse_command("/wallet").unwrap();
        assert_eq!(
            command,
            Command::Wallet {
                pubkey: None,
                label: None
            }
        );
    }

    #[test]
    fn test_parses_wallet_registration_with_label() {
        let command =
            parse_command("/wallet 7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU main").unwrap();
        assert_eq!(
            command,
            Command::Wallet {
                pubkey: Some("7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU".to_string()),
                label: Some("main".to_string()),
            }
        );
    }

    #[test]
    fn test_wallet_rejects_too_many_arguments() {
        assert!(parse_command("/wallet addr1 label1 extra").is_err());
    }

    #[test]
    fn test_parses_balance_without_wallet() {
        assert_eq!(
            parse_command("/balance").unwrap(),
            Command::Balance { wallet: None }
        );
    }

    #[test]
    fn test_parses_positions_with_wallet() {
        assert_eq!(
            parse_command("/positions addr1").unwrap(),
            Command::Positions {
                wallet: Some("addr1".to_string())
            }
        );
    }

    #[test]
    fn test_profit_requires_a_position_address() {
        assert!(parse_command("/profit").is_err());
    }

    #[test]
    fn test_parses_open_with_pool_and_width() {
        let command = parse_command("/open pool1 20").unwrap();
        assert_eq!(
            command,
            Command::Open {
                address: "pool1".to_string(),
                width: 20,
            }
        );
    }

    #[test]
    fn test_open_rejects_missing_width() {
        assert!(parse_command("/open pool1").is_err());
    }

    #[test]
    fn test_open_rejects_a_non_numeric_width() {
        assert!(parse_command("/open pool1 wide").is_err());
    }

    #[test]
    fn test_open_rejects_zero_width() {
        assert!(parse_command("/open pool1 0").is_err());
    }

    #[test]
    fn test_open_rejects_width_over_seventy() {
        assert!(parse_command("/open pool1 71").is_err());
    }

    #[test]
    fn test_open_accepts_the_maximum_width() {
        let command = parse_command("/open pool1 70").unwrap();
        assert_eq!(
            command,
            Command::Open {
                address: "pool1".to_string(),
                width: 70,
            }
        );
    }

    #[test]
    fn test_parses_add_with_two_amounts() {
        let command = parse_command("/add pos1 1.5 2.25").unwrap();
        assert_eq!(
            command,
            Command::Add {
                position: "pos1".to_string(),
                amount_x: Decimal::new(15, 1),
                amount_y: Decimal::new(225, 2),
            }
        );
    }

    #[test]
    fn test_add_rejects_a_non_numeric_amount() {
        assert!(parse_command("/add pos1 notanumber 2.0").is_err());
    }

    #[test]
    fn test_add_rejects_missing_amounts() {
        assert!(parse_command("/add pos1 1.0").is_err());
    }

    #[test]
    fn test_parses_remove_with_percent() {
        assert_eq!(
            parse_command("/remove pos1 50").unwrap(),
            Command::Remove {
                position: "pos1".to_string(),
                percent: 50,
            }
        );
    }

    #[test]
    fn test_remove_rejects_zero_percent() {
        assert!(parse_command("/remove pos1 0").is_err());
    }

    #[test]
    fn test_remove_rejects_percent_over_a_hundred() {
        assert!(parse_command("/remove pos1 101").is_err());
    }

    #[test]
    fn test_parses_claim() {
        assert_eq!(
            parse_command("/claim pos1").unwrap(),
            Command::Claim {
                position: "pos1".to_string()
            }
        );
    }

    #[test]
    fn test_parses_close() {
        assert_eq!(
            parse_command("/close pos1").unwrap(),
            Command::Close {
                position: "pos1".to_string()
            }
        );
    }

    #[test]
    fn test_close_rejects_missing_position() {
        assert!(parse_command("/close").is_err());
    }
}
