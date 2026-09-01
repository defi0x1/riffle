# riffle

A Solana backend that indexes [Meteora DLMM](https://docs.meteora.ag) liquidity pools, computes a
set of indicators over them at several timeframes, and serves the result through a read-only
Telegram bot.

It does not move funds. There are no keys anywhere in the process tree.

## Architecture

```
                    ┌──────────────┐
   Solana ─────────▶│   indexer    │────┐
   (RPC or Geyser)  └──────────────┘    │
                                        ▼
                                  ┌───────────┐      ┌──────────┐
                                  │ Postgres  │◀────▶│  scorer  │
                                  │ Timescale │      └──────────┘
                                  └───────────┘
                                        │
                                  ┌───────────┐
                                  │    bot    │  Telegram, read-only
                                  └───────────┘
```

| Binary | Role |
|---|---|
| `indexer` | Streams pool state and chain events from Solana (RPC or Geyser), decodes them, writes raw and rollup tables. Computes nothing. |
| `scorer` | Reads the raw tables, builds rollups, runs the indicator and ranking pipeline, emits signals, tracks paper positions. |
| `bot` | Read-only Telegram front end over the tables `scorer` writes. Decides nothing. |
| `crawler` | One-shot, operator-run backfill: walks transaction history for a pool set and range. Not started by any other binary. |

That separation is deliberate: **the bot must never be the thing that decides.** Every number it
shows was computed elsewhere, persisted with its inputs, and can be re-derived afterwards.

### Two ingestion backends, one trait

Ingestion sits behind a `Source` trait with two implementations, chosen by config:

| | RPC polling | Yellowstone Geyser |
|---|---|---|
| Cost | a normal RPC plan | a streaming endpoint on top |
| Latency | poll cadence | sub-second push |
| Pool state, active bin, active-bin liquidity | yes | yes |
| Swap-level detail -- signer, size, direction | no | yes |

Both write the same schema, so switching is a config change and a restart. The cheap backend runs
first; the expensive one earns its place by measurement rather than assumption. What it buys is
swap-level data, and therefore the ability to distinguish organic flow from arbitrage.

### Two-stage ranking

The ranking metric needs active-bin liquidity, which needs per-bin state, which is only affordable
for a watched subset. A system that could only rank pools it was already watching would never
discover anything, so ranking runs in two stages:

- **Screen** -- all pools, with active-bin liquidity estimated from TVL and a shape prior. Marked
  `quality = B`. Drives which pools get promoted to the watched set.
- **Rank** -- the watched set, with liquidity measured from real bin state. Marked `quality = A`.
  Only this feeds signals and outcome scoring.

Every row carries its quality and the bot renders it, because a ranking resting on a prior is a
materially weaker claim than a measured one. A slice of watched-set capacity is reserved for
never-measured pools, so a poor prior cannot permanently hide a pool from us.

### Multi-timeframe by construction

Each bucket stores pool **state** alongside traded **flow** -- price OHLC, TVL, active-bin
liquidity and the volatility accumulator, next to volume, fees and trade counts. Because both
halves are present per bucket, every indicator is computable at every timeframe rather than once
globally.

That makes the interesting comparison a single row lookup: a pool ranking high at one hour and low
at twenty-four is heating up; the reverse is decaying. A single blended score hides that
distinction.

## Running it

```sh
cp .env.example .env          # fill in RPC_URL
make up                       # postgres + timescale, prometheus, grafana
make migrate
cargo run --bin indexer -- --config config/indexer.example.yaml
```

Every setting is simultaneously a CLI flag, an environment variable of the same name, and a key in
a YAML file passed with `--config`. They resolve in that order -- a flag beats an environment
variable, which beats the file, which beats the built-in default. An unrecognised key in a config
file is an error naming the key, not a silent no-op.

`--help` on a built binary is the authoritative reference for one binary; `docs/` covers the rest:

- [`docs/configuration.md`](docs/configuration.md) -- every flag and environment variable, per binary.
- [`docs/operations.md`](docs/operations.md) -- bring-up order, health checks, alerts, recovery.
- [`docs/architecture.md`](docs/architecture.md) -- how ingestion, tiering and scoring fit together.

The calibration -- thresholds, weights and priors -- lives outside this repository;
`config/*.example.yaml` carries neutral placeholders.

## Layout

```
bin/
  indexer      ingestion workers: discovery, state, events, tiering, health
  scorer       rollups, indicators, signals, paper positions
  bot          Telegram commands, rendering, authorisation
  crawler      operator-run RPC backfill
libraries/
  common       config loading, worker trait, lifecycle
  logger       tracing setup
  metrics      prometheus registry and server
  source       the Source trait; rpc/ and geyser/ backends
  dlmm_decode  on-chain bytes to domain types
  dlmm_math    formulas; the only place fee and bin maths exists
  storage      schema, migrations, and every SQL query
  engine       volatility, regime, risk gate, ranking, sizing
migrations/    forward-only, numbered
observability/ prometheus scrape config, alert rules, grafana dashboards
tests/         end-to-end suite against a live database
```
