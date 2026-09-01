# Configuration

Every option below comes from a `clap::Parser` struct in the source tree. Each field with
`#[arg(long, env)]` produces:

- a flag: the field name in kebab-case (`database_url` -> `--database-url`)
- an environment variable: the field name in SCREAMING_SNAKE_CASE (`database_url` -> `DATABASE_URL`)
- a key in a YAML file passed with `--config <path>`, in the same snake_case as the field name
  (`database_url`)

Precedence, most to least authoritative: CLI flag > environment variable > config file > the
field's own compiled default -- a config file only fills in a value that neither a flag nor a real
environment variable already supplied. `--config` is deliberately CLI-only: its own path cannot
come from an environment variable or from the file it names. An unrecognised key in a config file
is an error naming the key, not a silent no-op. See `libraries/common/src/config.rs` for the
implementation.

Run `cargo run --bin <binary> -- --help` against a built binary for the authoritative,
always-current list of flags and environment variables -- this document is a guide to reading that
output, not a replacement for it.

## Secrets

These four values are credentials or would let a caller impersonate the service. None of them
have a default; all of them are gitignored wherever they end up on disk (`.env`, `config/*.yaml`
per `.gitignore`):

| Variable | Held by | Why it's a secret |
|---|---|---|
| `DATABASE_URL` | indexer, scorer, bot, crawler | Embeds the Postgres username and password. |
| `RPC_URL` | indexer (rpc backend), crawler | Most providers embed an API key in the URL path or query string. |
| `GEYSER_X_TOKEN` | indexer (geyser backend) | Bearer token for the Yellowstone gRPC stream. |
| `BOT_TOKEN` | bot | Telegram bot token from BotFather; anyone holding it can send messages as the bot and read its command traffic. |

Every `Display` impl in this codebase that could leak one of these is written by hand rather than
derived, specifically to keep it out of the startup log line (`PostgresConfig`, `source::Config`,
`telegram::Config` in `bin/bot/src/config.rs`). If you add a field that holds a secret, follow the
same pattern -- do not let `#[derive(Debug)]` cover a struct that also holds a credential.

## Shared groups (flattened into more than one binary)

### Logging (`logger::Config`) -- indexer, scorer, bot

| Flag / env | Type | Default | Required | Effect of a bad value |
|---|---|---|---|---|
| `--log-level` / `LOG_LEVEL` | string, `tracing_subscriber::EnvFilter` syntax | `info` | no | Fails to parse at startup (`Config::init` returns `Err`) and the process exits before doing anything; not a silent fallback. |
| `--log-format` / `LOG_FORMAT` | enum: `compact`, `full`, `json`, `pretty` | `compact` | no | Rejected by `clap` at parse time (unknown enum variant), process exits immediately. |

### Postgres (`common::PostgresConfig`) -- indexer, scorer, bot

| Flag / env | Type | Default | Required | Effect of a bad value |
|---|---|---|---|---|
| `--database-url` / `DATABASE_URL` | string, `postgres://user:pass@host/db` | none | **yes** | Missing: the process refuses to start (`clap` reports the missing required argument). Wrong host/credentials: the first `connect()` call fails with a wrapped error ("Connecting to postgres") and the process exits -- no retry loop, no degraded mode. |
| `--max-connections` / `MAX_CONNECTIONS` | u32 | `10` | no | Too low under real load serialises queries behind pool contention; too high can exceed Postgres's own `max_connections` once more than one binary is running against the same instance -- see [`docs/operations.md`](operations.md) for how the three binaries share this budget. |

### Metrics (`metrics::Config`) -- indexer, scorer, bot

| Flag / env | Type | Default | Required | Effect of a bad value |
|---|---|---|---|---|
| `--disable-metrics-server` / `DISABLE_METRICS_SERVER` | bool flag | `false` (server runs) | no | N/A -- a flag, not a value. |
| `--metrics-port` / `METRICS_PORT` | u16 | `9101` | no | **Every binary defaults to the same port.** Running indexer, scorer and bot on one host without overriding this on at least two of them fails the second and third binary's metrics server at bind time (`Binding metrics listener` error) -- the rest of the process still runs, only `/metrics` is down for that binary. `observability/prometheus/prometheus.yml` scrapes `9101` (indexer), `9102` (scorer), `9103` (bot); set `--metrics-port` to match on each binary you run together. |

There is no `--metrics-listen-addr` or bind-address override; the server always binds
`0.0.0.0:<port>`.

The `/metrics` server itself is generic across all three binaries (`metrics::Config::serve`
gathers whatever is registered in the shared process-wide registry), but only `indexer` actually
registers anything into it -- every collector in `libraries/metrics/src/ingest.rs`
(`processing_slot`, `rpc_call_total`, `ingest_lag_slots`, `decode_error_total`,
`stream_reconnect_total`) is populated by indexer's own workers, and neither `scorer` nor `bot`
registers anything of their own. Running `scorer --metrics-port 9102` or `bot --metrics-port 9103`
gives you a live, correctly-bound `/metrics` endpoint that returns an (almost) empty body -- this
is not a misconfiguration. Scorer's own operational state (pool counts by tier, rollup freshness,
signal counts) is surfaced through the Grafana dashboard's direct Postgres queries instead; see
[`docs/operations.md`](operations.md).

## `indexer`

### Top-level (`indexer::config::Args`)

| Flag / env | Type | Default | Required | Effect of a bad value |
|---|---|---|---|---|
| `--discovery-interval` / `DISCOVERY_INTERVAL` | duration (`humantime`) | `600s` | no | Shorter: more frequent full-universe scans, more RPC load from `getProgramAccounts`. Longer: newly created pools take longer to appear at all. |
| `--discovery-batch-size` / `DISCOVERY_BATCH_SIZE` | usize | `200` | no | Bounds how many brand-new pools get a full account fetch per discovery tick. Too high on a cold start against a large existing universe bursts RPC calls; too low means the backlog of undiscovered pools drains slowly. |
| `--state-flush-interval` / `STATE_FLUSH_INTERVAL` | duration | `10s` | no | How often buffered pool/bin state is written to Postgres regardless of batch size. Longer increases the window of data held only in memory (lost on a crash). |
| `--state-flush-batch-size` / `STATE_FLUSH_BATCH_SIZE` | usize | `50` | no | Buffered state updates that force an immediate flush before the timer fires. |
| `--event-flush-interval` / `EVENT_FLUSH_INTERVAL` | duration | `5s` | no | Same trade-off as `state-flush-interval`, for swap/liquidity events. |
| `--event-flush-batch-size` / `EVENT_FLUSH_BATCH_SIZE` | usize | `200` | no | Same trade-off as `state-flush-batch-size`. |
| `--health-interval` / `HEALTH_INTERVAL` | duration | `30s` | no | How often `ingest_health` gets a new row and the Prometheus gauges refresh. Shorter gives a finer-grained `/status`/dashboard view at the cost of one more write per tick. |
| `--health-freshness-threshold` / `HEALTH_FRESHNESS_THRESHOLD` | duration | `60s` | no | Data older than this no longer counts as "fresh" for the heartbeat log line (see [`docs/operations.md`](operations.md)). Set too low, a normal backend's own latency trips a false stale reading; too high delays real detection. |

### Tier (`indexer::config::TierConfig`)

Controls the two-stage screen/rank split described in [`docs/architecture.md`](architecture.md).

| Flag / env | Type | Default | Required | Effect of a bad value |
|---|---|---|---|---|
| `--max-watched` / `MAX_WATCHED` | i64 | `100` | no | Size of the tier-1 (measured) set. Too low starves the ranking stage of candidates; too high raises the `getMultipleAccounts` / subscription load per state-poll cycle roughly linearly. |
| `--promotion-interval` / `PROMOTION_INTERVAL` | duration | `300s` | no | How often tier membership is re-evaluated against the screening rank. Shorter reacts to a hot pool faster but adds a query load on `top_pools`/`watch_set` each sweep. |
| `--exploration-slice` / `EXPLORATION_SLICE` | f64, fraction in `[0,1]` | `0.10` | no | Share of watch-set slots reserved for never-measured pools regardless of screening rank. `0.0` means a poor screening prior can permanently hide a pool from ever being measured; values outside `[0,1]` are silently clamped by `TierWorker::tick` before use, not rejected. |
| `--demotion-margin` / `DEMOTION_MARGIN` | i64 | `20` | no | Extra rank slots below the cutoff a currently-watched pool may fall into before it is actually demoted (hysteresis, avoids flapping). `0` means any pool that drops even one slot below the strict cutoff is demoted on the next sweep. |

### Source backend (`source::Config`, `source::RpcConfig`, `source::GeyserConfig`)

| Flag / env | Type | Default | Required | Effect of a bad value |
|---|---|---|---|---|
| `--backend` / `BACKEND` | enum: `rpc`, `geyser` | `rpc` | no | Selects the whole ingestion implementation behind the `Source` trait -- see [`docs/architecture.md`](architecture.md). Unknown value is rejected by `clap` at parse time. |
| `--rpc-url` / `RPC_URL` | string | none | **yes when `backend=rpc`** | Missing: startup fails (`clap` required-argument error) when the rpc backend is selected -- the field is always declared required at the `RpcConfig` level, so it is required by `clap` even when `backend=geyser`, since `clap` does not do conditional-required across flattened groups. In practice: always set `RPC_URL`, whichever backend you run. Wrong/unreachable endpoint: every RPC call fails and retries per `max-retries`, then the tick logs an error and tries again next interval -- the process does not exit. |
| `--poll-interval-state` / `POLL_INTERVAL_STATE` | duration | `15s` | no | Poll cadence for tier-1 pool + bin state over RPC. Lower increases cost roughly linearly; the code comment notes the marginal benefit below 10s is small. |
| `--poll-interval-universe` / `POLL_INTERVAL_UNIVERSE` | duration | `600s` | no | Cadence of the full `getProgramAccounts` universe scan. This is the only place `gPA` is called; set it well above your provider's `gPA` rate limit or every scan fails. |
| `--datapi-url` / `DATAPI_URL` | string | `https://dlmm.datapi.meteora.ag` | no | Base URL for Meteora's public data API, the source of tier-0 flow metrics (volume/fees/TVL) on the rpc backend. Wrong URL: `flow_metrics` calls fail and tier-0 pools keep stale/absent flow data; does not crash the process. |
| `--datapi-page-size` / `DATAPI_PAGE_SIZE` | u32 | `500` | no | Page size for `/pools` requests; the API ignores `limit`/`per_page` beyond what has been observed to work. Larger than the API tolerates likely truncates or errors per-page, silently reducing universe coverage. |
| `--negative-cache-ttl` / `NEGATIVE_CACHE_TTL` | duration | `30s` | no | How long a "pool not present in datapi" result is cached before re-checking. Too long delays noticing a newly-listed pool; too short defeats the point of the cache (repeated full page walks for a pool that plainly does not exist yet). |
| `--max-concurrent-rpc` / `MAX_CONCURRENT_RPC` | usize | `8` | no | Cap on outbound RPC calls in flight. Too high risks provider rate-limiting or connection exhaustion; too low under-utilises the poll interval budget. |
| `--max-retries` / `MAX_RETRIES` | usize | `5` | no | Retry ceiling per failed RPC/datapi call before the tick gives up and logs an error. `0` means any transient failure is treated as a tick failure immediately. |
| `--geyser-endpoint` / `GEYSER_ENDPOINT` | string, optional | none | **yes when `backend=geyser`** | Missing while `backend=geyser`: `GeyserSource::new` fails at construction (fails fast at startup, not on first stream use) because `ConnectionConfig::new` requires it. |
| `--geyser-x-token` / `GEYSER_X_TOKEN` | string, optional | none | usually yes (provider-dependent) | Missing or wrong: the gRPC connection is refused/unauthenticated when the stream actually opens; `GeyserSource::new` itself does not validate the token, only that a value is well-formed enough to attach. |
| `--geyser-commitment` / `GEYSER_COMMITMENT` | string | `confirmed` | no | Parsed at construction (`filters::parse_commitment`); an unrecognised value fails `GeyserSource::new` immediately, so the process never starts with an invalid commitment level. |

Note the naming: the field is `geyser_x_token`, so the flag is `--geyser-x-token` and the
environment variable is `GEYSER_X_TOKEN` -- not `GEYSER_TOKEN`.

## `scorer`

`scorer`'s library code (the pipeline, rollup and worker-state modules under `bin/scorer/src/`)
implements the tick logic described in [`docs/architecture.md`](architecture.md) and is unit
tested. `bin/scorer/src/main.rs` constructs and runs all four workers (rollup, indicators,
signals, paper positions) under the same `common::run_workers` supervision every other binary
uses, and connects to Postgres and runs migrations on startup exactly like `indexer` does. The
configuration below is live and `--help` against a built binary is authoritative for it, the same
as for `indexer` and `bot`.

### Tick intervals (`scorer::config::TickConfig`)

| Flag / env | Type | Default | Required | Effect of a bad value |
|---|---|---|---|---|
| `--rollup-interval` / `ROLLUP_INTERVAL` | duration | `5m` | no | How often `pool_metrics_5m`/`pool_metrics_10m` buckets are built from raw tables. Must stay in step with the 5-minute bucket width baked into the rollup worker's own bucket-flooring logic; changing it does not change the bucket width, only how often a (possibly incomplete) bucket is (re)built. |
| `--indicators-interval` / `INDICATORS_INTERVAL` | duration | `5m` | no | How often the universe is screened and the watch set ranked. |
| `--signals-interval` / `SIGNALS_INTERVAL` | duration | `5m` | no | How often trigger conditions are re-evaluated for signal emission. |
| `--signal-cooldown` / `SIGNAL_COOLDOWN` | duration | `1h` | no | Minimum gap before a persistent signal condition is re-announced. `0` would re-announce every tick a condition still holds. |
| `--paper-position-mark-interval` / `PAPER_POSITION_MARK_INTERVAL` | duration | `5m` | no | How often open paper positions are marked against real fee accrual. |
| `--outcomes-interval` / `OUTCOMES_INTERVAL` | duration | `15m` | no | How often the outcomes worker checks for positions whose horizon has come due. Two horizons run today, 24h and 72h; a third 14-day horizon for the S regime is named in the `outcomes` table's migration comment but not implemented in `PaperPositionWorker` (`OUTCOME_HORIZONS` in `bin/scorer/src/paper/worker.rs` has only the two). |

### Pipeline defaults (`scorer::config::PipelineDefaultsConfig`)

Neutral placeholders for inputs the engine needs but does not calibrate itself; the calibrated
values live outside this repository.

| Flag / env | Type | Default | Required | Effect of a bad value |
|---|---|---|---|---|
| `--kappa-c` / `KAPPA_C` | f64 | `3.0` | no | Fee-clustering multiplier feeding the endogenous fee forecast. Miscalibrated, the forecast fee (`f_hat`) is systematically over- or under-stated, which propagates into `r_org` and every ranking/sizing decision downstream. |
| `--decay-window-secs` / `DECAY_WINDOW_SECS` | f64 (seconds) | `600.0` | no | Decay window for the forecast-fee volatility term. |
| `--organic-class-prior-mu` / `ORGANIC_CLASS_PRIOR_MU` | f64 | `0.6` | no | Shrinkage prior mean for the organic-flow blend, until per-class priors are estimated. |
| `--organic-class-prior-tau-sq` / `ORGANIC_CLASS_PRIOR_TAU_SQ` | f64 | `0.05` | no | Shrinkage prior variance for the same blend. |
| `--regime-capital` / `REGIME_CAPITAL` | f64 (USD) | `1_000_000.0` | no | Capital assumed available to the current regime bucket for sizing. No portfolio ledger exists yet -- this is a flat placeholder, not a tracked balance. |
| `--free-capital` / `FREE_CAPITAL` | f64 (USD) | `200_000.0` | no | Capital assumed uncommitted, for sizing. |
| `--mu-fee` / `MU_FEE` | f64 | `0.001` | no | Expected fee return feeding quarter-Kelly sizing. |
| `--mu-arb` / `MU_ARB` | f64 | `0.0002` | no | Expected adverse-selection cost feeding quarter-Kelly sizing. |

None of these eight are validated for sign or range at parse time; a nonsensical value (a negative
`decay_window_secs`, say) is accepted by `clap` and produces nonsensical or `NaN`-poisoned
downstream numbers rather than a startup error. There is no runtime bounds check in this codebase
today.

### Engine (`engine::EngineConfig`)

Six flattened sub-groups, all neutral placeholders -- the calibrated thresholds live outside this
repository, as the docstring on every one of these structs says explicitly. None are individually
required; `clap` accepts any `f64`/`u16`/`u32`/duration for every field below, so an out-of-range
value (a negative fraction, a threshold above 1.0 where the field is meant to be a share) is not
rejected at startup. Its effect is confined to the decision pipeline: a wrong gate threshold passes
pools that should fail (or the reverse), a wrong persistence window makes the exit-trigger logic
flap or freeze -- never a crash, always a silently different ranking.

Per-regime fields follow a fixed naming pattern: `_s` (Stable), `_v1` (established volatile),
`_v2` (young/volatile) suffixes on the same underlying threshold.

**Regime classifier** (`--group regime`, `RegimeConfig`) -- hysteresis and the S/V1/V2 boundary conditions:

| Flag / env | Default | What it gates |
|---|---|---|
| `--persistence` / `PERSISTENCE` | `30m` | Time a candidate regime must hold before the classifier commits to it. |
| `--cooldown` / `COOLDOWN` | `2h` | Minimum time between committed regime flips (bypassed by the kill switch). |
| `--s-enter-sigma-slow-max` / `S_ENTER_SIGMA_SLOW_MAX` | `0.005` | S-enter: `sigma_slow` ceiling. |
| `--s-enter-dev-peg-max` / `S_ENTER_DEV_PEG_MAX` | `0.003` | S-enter: peg deviation ceiling. |
| `--s-exit-sigma-fast-min` / `S_EXIT_SIGMA_FAST_MIN` | `0.01` | S-exit: `sigma_fast` floor. |
| `--s-exit-dev-peg-min` / `S_EXIT_DEV_PEG_MIN` | `0.005` | S-exit: peg deviation floor. |
| `--v2-enter-sigma-slow-min` / `V2_ENTER_SIGMA_SLOW_MIN` | `0.08` | V2-enter: `sigma_slow` floor. |
| `--v2-enter-age-max-days` / `V2_ENTER_AGE_MAX_DAYS` | `30.0` | V2-enter: age ceiling in days. |
| `--v2-exit-sigma-slow-max` / `V2_EXIT_SIGMA_SLOW_MAX` | `0.05` | V2-exit: `sigma_slow` ceiling. |
| `--v2-exit-age-min-days` / `V2_EXIT_AGE_MIN_DAYS` | `30.0` | V2-exit: age floor in days. |

**Risk gate** (`--group risk_gate`, `RiskGateConfig`) -- runs before any attractiveness metric; a failing pool is not scored at all:

| Flag / env | Default | What it gates |
|---|---|---|
| `--top10-holder-share-max` / `TOP10_HOLDER_SHARE_MAX` | `0.35` | Top-10 holder share ceiling (when holder data is available). |
| `--top1-holder-share-max` / `TOP1_HOLDER_SHARE_MAX` | `0.15` | Single-wallet holder share ceiling. |
| `--transfer-fee-bps-max` / `TRANSFER_FEE_BPS_MAX` | `100` (u16) | Token-2022 transfer-fee ceiling, in bps. |
| `--other-venue-min-depth-ratio` / `OTHER_VENUE_MIN_DEPTH_RATIO` | `0.20` | Minimum depth on another venue/CEX relative to DLMM depth. |
| `--creator-fee-change-window` / `CREATOR_FEE_CHANGE_WINDOW` | `7d` | A base-fee change inside this window fails the creator-behaviour check. |
| `--wash-signer-volume-share-max` / `WASH_SIGNER_VOLUME_SHARE_MAX` | `0.40` | Ceiling on 24h volume share from the top wash-screen signers. |
| `--wash-signer-count` / `WASH_SIGNER_COUNT` | `5` (u32) | Number of signers the wash-screen share is measured over. |
| `--wash-round-trip-ratio-max` / `WASH_ROUND_TRIP_RATIO_MAX` | `0.30` | Round-trip volume ratio ceiling. |
| `--v2-min-age` / `V2_MIN_AGE` | `72h` | Minimum pool age for V2, since first liquidity. |

**Organic flow** (`--group organic_flow`, `OrganicFlowConfig`):

| Flag / env | Default | What it gates |
|---|---|---|
| `--default-sample-variance` / `DEFAULT_SAMPLE_VARIANCE` | `0.05` | Assumed sample variance of the observed organic share, sizing the shrinkage weight until a class's own dispersion is estimated. |

**Ranking** (`--group ranking`, `RankingConfig`) -- the attractiveness gate, one triple per regime unless noted:

| Flag / env | Defaults (S / V1 / V2) | What it gates |
|---|---|---|
| `--r-min-s` / `--r-min-v1` / `--r-min-v2` | `1.5` / `2.0` / `3.0` | Minimum `R` (fee-over-risk ratio) to be considered attractive. Sits above the derived breakeven by design -- a model-error budget, not the breakeven itself. |
| `--vol-tvl-min-s` / `-v1` / `-v2` | `1.0` / `1.5` / `4.0` | Minimum 24h volume / TVL ratio. |
| `--phi-org-min-s` / `-v1` / `-v2` | `0.50` / `0.40` / `0.50` | Minimum organic-flow fraction. Documented as "locked, not tuned" in source. |
| `--y-fee-annual-min-s` / `-v1` / `-v2` | `0.08` / `0.25` / `1.50` | Minimum annualised fee yield. |
| `--tvl-min-s` / `-v1` / `-v2` | `1_000_000` / `500_000` / `150_000` | Minimum TVL, USD. |
| `--vol24h-min-s` / `-v1` / `-v2` | `2_000_000` / `1_000_000` / `500_000` | Minimum 24h volume, USD. |
| `--volume-trend-min-wk-wk` / `VOLUME_TREND_MIN_WK_WK` | `-0.50` | S/V1: week-over-week volume trend floor (fraction, negative). |
| `--volume-trend-min-v2-young` / `VOLUME_TREND_MIN_V2_YOUNG` | `0.35` | V2, age < 7d: 24h volume as a fraction of the trailing 72h average. |
| `--volume-trend-min-v2-mature` / `VOLUME_TREND_MIN_V2_MATURE` | `0.50` | V2, age >= 7d: same ratio, higher bar. |
| `--h-jit-s` / `-v1` / `-v2` | `0.05` / `0.10` / `0.15` | JIT-liquidity haircut applied inside `R_org`. |
| `--ranking-key-hurdle-annual` / `RANKING_KEY_HURDLE_ANNUAL` | `0.20` | Annual hurdle sizing the ranking key's dilution-adjusted position cap. |
| `--consistency-min-ratio` / `CONSISTENCY_MIN_RATIO` | `0.5` | Multi-window consistency filter: the minimum of the 1h/24h/7d fee/TVL windows must reach this fraction of the 24h window. |

**Sizing** (`--group sizing`, `SizingConfig`):

| Flag / env | Defaults (S / V1 / V2) | What it gates |
|---|---|---|
| `--theta-max-s` / `-v1` / `-v2` | `0.15` / `0.10` / `0.08` | Position-share-of-active-liquidity cap. |
| `--pi-max-s` / `-v1` / `-v2` | `0.10` / `0.05` / `0.03` | Position-share-of-TVL cap. |
| `--car-fraction-s` / `-v1` / `-v2` | `0.40` / `0.20` / `0.10` | Capital-at-risk fraction cap. |
| `--v-min-s` / `-v1` / `-v2` | `5_000` / `3_000` / `1_000` | Minimum position size, USD, below which a pool is skipped entirely. |
| `--annual-hurdle` / `ANNUAL_HURDLE` | `0.20` | Annual hurdle used to compute the self-dilution cap. |
| `--position-count` / `POSITION_COUNT` | `5` (u32) | Position count `N` for Spot sizing (`V = N * m`). |

**Triggers** (`--group triggers`, `TriggersConfig`) -- exit conditions, each a persistence-window lookback rather than a stateful accumulator:

| Flag / env | Defaults (S / V1 / V2) | What it gates |
|---|---|---|
| `--r-org-exit-s` / `-v1` / `-v2` + matching `*-persistence-*` | `1.0`/`24h`, `1.5`/`6h`, `2.0`/`3h` | `R_org` exit threshold and how long it must hold. |
| `--vol-tvl-exit-s` / `-v1` / `-v2` + matching `*-persistence-*` | `0.5`/`48h`, `0.75`/`12h`, `2.0`/`6h` | Volume/TVL exit threshold and persistence. |
| `--volume-decay-wk-wk-min` / `VOLUME_DECAY_WK_WK_MIN` | `-0.50` | S: week-over-week volume-decay exit floor. |
| `--volume-decay-v1-min` / `VOLUME_DECAY_V1_MIN` | `0.40` | V1: 24h/trailing-72h-average volume floor. |
| `--volume-decay-v2-young-min` / `VOLUME_DECAY_V2_YOUNG_MIN` | `0.35` | V2, age < 7d: same ratio. |
| `--volume-decay-v2-mature-min` / `VOLUME_DECAY_V2_MATURE_MIN` | `0.50` | V2, age >= 7d: same ratio, higher bar. |
| `--fee-jack-kill-multiplier` / `FEE_JACK_KILL_MULTIPLIER` | `2.0` | A fee-parameter jump at or above this multiplier is an instant kill, no persistence window. |

## `bot`

### Telegram (`bot::config::Config`, `#[group(id = "telegram")]`)

| Flag / env | Type | Default | Required | Effect of a bad value |
|---|---|---|---|---|
| `--bot-token` / `BOT_TOKEN` | string | none | **yes** | Missing: startup fails (`clap` required-argument error). Present but wrong: every Telegram API call the bot makes fails; `teloxide`'s polling loop logs the error per the update-listener error path in `TelegramWorker::run` and keeps retrying rather than exiting. |
| `--allowed-chats` / `ALLOWED_CHATS` | `Vec<i64>`, comma-separated | none | **yes** | Missing: startup fails. Empty list (`ALLOWED_CHATS=`): every chat is refused (`is_authorized` returns `false` for all input when the list is empty), so the bot runs but answers nothing -- not a crash, a silent no-op from the operator's point of view. |
| `--max-rows` / `MAX_ROWS` | usize | `10` | no | Rows rendered per ranking command before pagination. Very large values risk hitting Telegram's message-length pagination path more often per command (more API calls, slower replies) rather than failing outright. |

Note the naming here too: the flags are `--bot-token` / `--allowed-chats`, not
`--telegram-bot-token` / `--telegram-allowed-chats` -- `#[group(id = "telegram")]` only affects how
`--help` groups the arguments, it does not prefix the flag or environment-variable names.

## `crawler`

A one-shot, operator-run RPC backfill: walks `getSignaturesForAddress` backward over a pool set
and range, decodes each transaction's swap/liquidity events, and writes them into the same
`swaps`/`liquidity_events` tables `indexer` writes -- not a long-running worker, and not started by
any other binary. It is `common::PostgresConfig` and `logger::Config` plus three groups of its own.

### RPC (`crawler::cli::RpcConfig`, `#[group(id = "crawler-rpc")]`)

| Flag / env | Type | Default | Required | Effect of a bad value |
|---|---|---|---|---|
| `--rpc-url` / `RPC_URL` | string | none | **yes** | Missing: startup fails (`clap` required-argument error). Wrong/unreachable endpoint: every call fails and retries per `max-retries`, then that pool's walk aborts with an error -- the process does not silently skip a pool. |

### Pacing (`crawler::pacing::PacingConfig`, `#[group(id = "pacing")]`)

| Flag / env | Type | Default | Required | Effect of a bad value |
|---|---|---|---|---|
| `--max-concurrent-rpc` / `MAX_CONCURRENT_RPC` | usize | `4` | no | Outbound RPC calls in flight at once. Lower than `indexer`'s own default (8), since a backfill is a burst of `getTransaction` calls per page rather than a steady poll, and is more likely to trip a provider's rate limit if run too wide. |
| `--min-request-interval` / `MIN_REQUEST_INTERVAL` | duration | `150ms` | no | Minimum spacing between the *starts* of two calls, independent of concurrency -- the primary throttle against a provider's rate limit. Too low against a strict provider produces 429s that eat into `max-retries`; too high makes a large backfill take proportionally longer. |
| `--max-retries` / `MAX_RETRIES` | usize | `6` | no | Retry attempts for one failed call before the walk gives up on that pool entirely and returns an error. |
| `--backoff-base` / `BACKOFF_BASE` | duration | `500ms` | no | Delay before the first retry; doubles each subsequent attempt up to `backoff-max`. |
| `--backoff-max` / `BACKOFF_MAX` | duration | `20s` | no | Ceiling on the retry delay. |

### Range (`crawler::cli::RangeConfig`, `#[group(id = "crawler-range")]`)

| Flag / env | Type | Default | Required | Effect of a bad value |
|---|---|---|---|---|
| `--pools` / `POOLS` | comma-separated addresses | none | see note | Combined with `--pools-file` (deduplicated, `--pools` entries sort first) into the address list to backfill. |
| `--pools-file` / `POOLS_FILE` | path | none | see note | Newline-delimited pool addresses; blank lines and `#`-prefixed lines are ignored. **At least one of `--pools`/`--pools-file` must resolve to a non-empty list** -- this is checked at runtime (`resolve_pools`), not by `clap`, so an empty combination fails with an explicit error after startup, not a `clap` required-argument error. A malformed address in either source fails the same way. |
| `--from-slot` / `FROM_SLOT` | u64, optional | none (walk as far back as the node retains) | no | Lower slot bound, inclusive. |
| `--to-slot` / `TO_SLOT` | u64, optional | none (start from current chain head) | no | Upper slot bound, inclusive. |
| `--from-time` / `FROM_TIME` | RFC 3339 timestamp, optional | none | no | Lower time bound. Malformed input is rejected by `clap`'s value parser at startup, not accepted and ignored. |
| `--to-time` / `TO_TIME` | RFC 3339 timestamp, optional | none | no | Upper time bound. |
| `--page-size` / `PAGE_SIZE` | usize | `1000` | no | Signatures requested per `getSignaturesForAddress` page; the RPC method itself caps this at 1000, so a larger value is accepted by `clap` but has no further effect. |
| `--write-batch-size` / `WRITE_BATCH_SIZE` | usize | `200` | no | Buffered rows written per batch, independent of page size. |

### Top-level (`crawler::cli::Args`)

| Flag / env | Type | Default | Required | Effect of a bad value |
|---|---|---|---|---|
| `--checkpoint-file` / `CHECKPOINT_FILE` | path | `crawler_checkpoint.json` | no | Where per-pool progress (the oldest signature reached, plus counters) is recorded after every page, so an interrupted run resumes near where it stopped rather than re-walking and re-paying for transactions already fetched. A missing file is treated as a fresh start, not an error. Correctness never depends on this file -- every write is idempotent on `(pool_address, ts, signature, ix_index)`, the same key the live indexer path uses -- so deleting it and re-running is always safe, just slower. |
| `--dry-run` / `DRY_RUN` | bool flag | `false` | no | Walks the range and reports what would be fetched, touching neither Postgres nor the checkpoint file. Does not call `storage::run_migrations`, so it also works against a database that does not exist yet. |

Like `indexer` and `scorer`, `crawler` calls `storage::run_migrations` on startup (skipped
entirely under `--dry-run`), so it does not need `make migrate` to have run first either.

`crawler` writes only `swaps` and `liquidity_events` from decoded transaction history --
it has no path to backfill `pool_snapshots`/`dlmm_pool_state`/`bin_states`/`active_bin_snapshots`,
since those need account *state* at a past slot, which a signature walk cannot reconstruct without
an archival RPC node. See [`docs/operations.md`](operations.md) for how this fits into recovering
an ingestion gap.

## Known gaps between configuration and its documentation

Found while writing this document, listed here rather than silently corrected, since fixing them
is out of scope for a documentation pass:

- **No environment variable is picked up from `.env` automatically by any Rust binary.** `.env` is
  read automatically by `sqlx-cli` (so `make migrate` works after `cp .env.example .env`), but
  `indexer`, `scorer` and `bot` read only the real process environment -- there is no `dotenvy` (or
  similar) call anywhere in this workspace. Export the variables into your shell, pass them as
  `VAR=value cargo run ...`, use a `--config` file, or use `docker compose`'s `env_file:` if you
  containerise these binaries; a bare `.env` file sitting in the working directory does nothing for
  them on its own.
