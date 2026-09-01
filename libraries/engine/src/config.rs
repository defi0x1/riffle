use clap::Parser;

use crate::organic_flow::OrganicFlowConfig;
use crate::ranking::RankingConfig;
use crate::regime::RegimeConfig;
use crate::risk_gate::RiskGateConfig;
use crate::sizing::SizingConfig;
use crate::triggers::TriggersConfig;

/// Every threshold the decision pipeline uses, composed from each stage's own config so a
/// binary can flatten them together with its other subsystems' configs. Ships with
/// neutral defaults; the calibrated values live outside this repo.
#[derive(Parser, Debug, Clone)]
pub struct EngineConfig {
    #[clap(flatten)]
    pub regime: RegimeConfig,
    #[clap(flatten)]
    pub risk_gate: RiskGateConfig,
    #[clap(flatten)]
    pub organic_flow: OrganicFlowConfig,
    #[clap(flatten)]
    pub ranking: RankingConfig,
    #[clap(flatten)]
    pub sizing: SizingConfig,
    #[clap(flatten)]
    pub triggers: TriggersConfig,
}
