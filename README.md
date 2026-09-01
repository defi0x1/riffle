# fee-farming

A Solana backend that indexes [Meteora DLMM](https://docs.meteora.ag) liquidity pools and ranks
them by how profitable they are to provide liquidity to — served through a read-only Telegram bot.

It does not move funds. There are no keys anywhere in the process tree.

## The question it answers

Most pool rankings answer *"what is busy right now"* — fees earned, volume, price change. That is
a reasonable question and several tools answer it well.

This answers a different one: **which pools earn more in fees than they lose to arbitrageurs.**

A liquidity provider in a constant-function market maker is short volatility. Every time the price
moves, an arbitrageur rebalances the pool at a stale price and takes the difference. The formal
name for that cost is Loss-Versus-Rebalancing (Milionis, Moallemi, Roughgarden & Zhang, 2022), and
for a concentrated position it is proportional to `σ²`. Fee income, meanwhile, is proportional to
volume through the active range.

So the metric that matters is a ratio:

```
R = 2 · f · τ · s · (1 − ps) / σ²
```

where `f` is the fee rate, `τ` the turnover through the active bin, `s` the bin step, `ps` the
protocol's share and `σ` the daily volatility. Breakeven is `R = 1`. The useful property is that
**`R` is independent of position size, range width and shape** — so "which pool" is answerable
separately from "how much", and the ranking is a property of the pool rather than of a particular
position in it.

A pool ranked highly on activity but poorly on `R` is a crowded, volatile trap. The reverse is a
quiet pool nobody is looking at. The disagreements are the interesting output.

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

Four binaries. `indexer` streams and decodes and computes nothing. `scorer` computes every
indicator and writes each signal with the reasoning that produced it. `bot` reads and renders and
decides nothing. `crawler` is an operator-run tool for repairing a gap in the record.

That separation is deliberate: **the bot must never be the thing that decides.** Every number it
shows was computed elsewhere, persisted with its inputs, and can be re-derived afterwards.

### Two ingestion backends, one trait

Ingestion sits behind a `Source` trait with two implementations, chosen by config:

| | RPC polling | Yellowstone Geyser |
|---|---|---|
| Cost | a normal RPC plan | a streaming endpoint on top |
| Latency | poll cadence | sub-second push |
| Pool state, active bin, active-bin liquidity | yes | yes |
| Swap-level detail — signer, size, direction | no | yes |

Both write the same schema, so switching is a config change and a restart. The cheap backend runs
first; the expensive one earns its place by measurement rather than assumption. What it buys is
swap-level data, and therefore the ability to distinguish organic flow from arbitrage — which is
the difference between a good estimate of `R` and a great one.

### Two-stage ranking

`R` needs active-bin liquidity, which needs per-bin state, which is only affordable for a watched
subset. A system that could only rank pools it was already watching would never discover anything,
so ranking runs in two stages:

- **Screen** — all pools, with active-bin liquidity estimated from TVL and a shape prior. Marked
  `quality = B`. Drives which pools get promoted to the watched set.
- **Rank** — the watched set, with liquidity measured from real bin state. Marked `quality = A`.
  Only this feeds signals and outcome scoring.

Every row carries its quality and the bot renders it, because a ranking resting on a prior is a
materially weaker claim than a measured one. A slice of watched-set capacity is reserved for
never-measured pools, so a poor prior cannot permanently hide a pool from us.

### Multi-timeframe by construction

Each bucket stores pool **state** alongside traded **flow** — price OHLC, TVL, active-bin
liquidity and the volatility accumulator, next to volume, fees and trade counts. Because both
halves are present per bucket, every indicator is computable at every timeframe rather than once
globally.

That makes the interesting comparison a single row lookup: a pool with a high `R` at one hour and
a low `R` at twenty-four is heating up; the reverse is decaying. A single blended score hides that
distinction.

## Extensibility

Three seams exist because three specific changes are anticipated. Everything else is flat code.

- **A second venue.** The ranking is written once, through a `Venue` trait, because the underlying
  algebra is genuinely shared — a bin-based AMM and a ranged constant-product AMM produce the same
  dimensionless fee-over-LVR ratio, differing only in a geometry term. Pool tables are split into a
  venue-agnostic core plus per-venue satellites, so adding one is `CREATE TABLE` and new rows, never
  an `ALTER` of a populated table.
- **A second consumer.** All SQL lives in one crate; the bot contains none. Adding an HTTP API is a
  serialisation layer over functions that already exist and are already tested.
- **A second ingestion source.** The `Source` trait above.

Deliberately absent: a plugin registry, a database abstraction layer, a message broker between
processes, and multi-chain generics. Each was considered and rejected for want of a concrete use.

## Running it

```sh
cp .env.example .env          # fill in RPC_URL and TELEGRAM_BOT_TOKEN
make up                       # postgres + timescale, prometheus, grafana
make migrate
cargo run --bin indexer -- --help
```

Configuration is clap-derived, so every setting is simultaneously a flag, an environment variable
and a YAML key, and `--help` is the reference. Real config files are gitignored; the tracked
`config/*.example.yaml` carry neutral values.

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

## Status

Under construction. The calibration — thresholds, weights and priors — lives outside this
repository; `config/*.example.yaml` carries neutral placeholders.

## License

Not yet licensed.
