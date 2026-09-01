# fee-farming

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

| Binary | Role | Status |
|---|---|---|
| `indexer` | Streams pool state and chain events from Solana (RPC or Geyser), decodes them, writes raw + rollup tables. Computes nothing. | Runs |
| `scorer` | Reads the raw tables, builds rollups, runs the indicator/ranking pipeline, emits signals, tracks paper positions. | Runs |
| `bot` | Read-only Telegram front end over the tables `scorer` writes. Decides nothing. | Runs |
| `crawler` | One-shot, operator-run backfill: walks transaction history for a pool set and range and writes the swap/liquidity events into it. Not started by any other binary. | Runs |

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
swap-level data, and therefore the ability to distinguish organic flow from arbitrage -- which is
the difference between a good estimate of `R` and a great one.

### Two-stage ranking

`R` needs active-bin liquidity, which needs per-bin state, which is only affordable for a watched
subset. A system that could only rank pools it was already watching would never discover anything,
so ranking runs in two stages:

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

That makes the interesting comparison a single row lookup: a pool with a high `R` at one hour and
a low `R` at twenty-four is heating up; the reverse is decaying. A single blended score hides that
distinction.

## Extensibility

Three seams exist because three specific changes are anticipated. Everything else is flat code.

- **A second venue.** Ranking is written once, against a `Venue` trait, because the underlying
  algebra is genuinely shared between concentrated-liquidity designs -- they differ by a geometry
  term, not by structure. Pool tables are split into a venue-agnostic core plus per-venue
  satellites, so adding one is `CREATE TABLE` and new rows, never an `ALTER` of a populated table.
- **A second consumer.** All SQL lives in one crate; the bot contains none. Adding an HTTP API is a
  serialisation layer over functions that already exist and are already tested.
- **A second ingestion source.** The `Source` trait above.

Deliberately absent: a plugin registry, a database abstraction layer, a message broker between
processes, and multi-chain generics. Each was considered and rejected for want of a concrete use.

## Running it

Building requires a sibling checkout of Meteora's `DLMM` program repository at `../DLMM` relative
to this one (`lb_clmm` is a local path dependency).

```sh
cp .env.example .env          # fill in RPC_URL; the bot vars need renaming, see below
make up                       # postgres + timescale, prometheus, grafana
make migrate
cargo run --bin indexer -- --help
```

Configuration is clap-derived, so every setting is simultaneously a CLI flag and an environment
variable of the same name, and `--help` against a built binary is the reference -- see
[`docs/configuration.md`](docs/configuration.md) for the full list, including three variable-name
mismatches in `.env.example` worth knowing about before you copy it (`BOT_TOKEN`/`ALLOWED_CHATS`,
not `TELEGRAM_BOT_TOKEN`/`TELEGRAM_ALLOWED_CHATS`; `GEYSER_X_TOKEN`, not `GEYSER_TOKEN`). No binary
in this workspace reads a YAML config file today; `config/*.example.yaml` is illustrative only.

For the full bring-up sequence (all three binaries, in the order that makes sense, plus how to
tell whether ingestion is healthy) see [`docs/operations.md`](docs/operations.md).

## Layout

```
bin/         indexer, scorer, bot, crawler
libraries/
  common     config, worker trait, lifecycle
  logger     tracing setup
  metrics    prometheus registry and server
  source     the Source trait; rpc/ and geyser/ backends
  dlmm_decode  on-chain bytes to domain types
  dlmm_math    formulas; the only place fee and bin maths exists
  storage    schema, migrations, and every SQL query
  engine     volatility, regime, risk gate, ranking, sizing
migrations/  forward-only, numbered
```

## Notes on correctness

A few things that are easy to get wrong and are handled deliberately:

- **Account groups are read in one batch.** `getMultipleAccounts` caps at 100 keys; chunking
  naively reads a pool in one batch and its bin arrays in the next, at different slots. Groups are
  bin-packed so a pool and its dependents are always read at a single slot.
- **Time comes from the chain.** The Clock sysvar is appended to every batch, so each read carries
  its own authoritative slot and timestamp. The dynamic-fee accumulator decays against on-chain
  time, never wall clock.
- **The event's fee field includes the protocol cut.** The LP share is `fee − protocol_fee`.
  Getting this wrong overstates revenue everywhere downstream, silently.
- **Protocol parameters are read per pool, never assumed.** Fee factors, the accumulator
  parameters and the protocol share are per-pool account fields.
- **Decoding is pinned by golden tests** against real mainnet account bytes, so a program upgrade
  that shifts a layout fails the build rather than corrupting the record.

## Documentation

- [`docs/configuration.md`](docs/configuration.md) -- every flag and environment variable, per binary.
- [`docs/operations.md`](docs/operations.md) -- the runbook: bring-up order, health checks, alerts, recovery.
- [`docs/architecture.md`](docs/architecture.md) -- how ingestion, tiering and the scoring pipeline fit together.

## Status

Under construction. The calibration -- thresholds, weights and priors -- lives outside this
repository; `config/*.example.yaml` carries neutral placeholders.

## License

Not yet licensed.
