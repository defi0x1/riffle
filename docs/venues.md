# Venues

This document describes how the system generalises -- or does not yet generalise -- across
automated market makers. It is written from the code as it exists today, not from intent. One
venue is implemented: Meteora DLMM, a bin-based concentrated-liquidity AMM. The abstractions this
document describes were built with three more venues in mind -- Meteora DAMM v2, Raydium CLMM,
Orca Whirlpool -- but none of the three has a line of code behind it. Where this document says
"would," it means exactly that: nothing has been tried, only designed for.

The short version of the claim the code makes: ranking is written once, against a small trait,
because the fee/risk algebra behind any concentrated-liquidity AMM is the same expression with one
term -- a geometry factor -- substituted per venue. That claim holds for the ranking math itself.
It does not extend as far as the surrounding system's comments imply: the query layer that feeds
the ranking math a pool's parameters is hardwired to DLMM's tables today, and one of the pipeline
stages (`organic_flow`) reads a DLMM-specific geometry value directly, bypassing the trait
entirely. Both are pointed out below, with file and line, alongside what does generalise cleanly.

## 1. What is actually shared

Two axes get confused easily, so this document keeps them separate throughout:

- **Ingestion** -- how data arrives: RPC polling vs. a Geyser account/transaction stream. This is
  the `Source` trait in `libraries/source/src/lib.rs`, and it has nothing to do with which AMM
  produced the data.
- **Venue** -- which AMM produced the data, and what its liquidity geometry is. This is the
  `Venue` trait in `libraries/dlmm_math/src/ranking.rs`.

A pool's `Source` backend and its venue vary independently: a DAMM v2 pool would still be read over
either RPC or Geyser, using whichever `Source` impl is already running.

### The metrics that mean the same thing on any AMM

Once a pool's state is reduced to a handful of scalars -- a fee rate, an amount of liquidity that
sets the turnover denominator, a volatility estimate, a TVL figure -- the rest of the decision
pipeline in `libraries/engine/src/` does not care which program produced them. Concretely, these
stages take only already-reduced scalar inputs and never touch `dlmm_math::PoolState` or the
`Venue` trait:

| Stage | File | Input shape |
|---|---|---|
| `volatility::evaluate` | `libraries/engine/src/volatility.rs` | OHLC bars, log returns |
| `risk_gate::evaluate` | `libraries/engine/src/risk_gate.rs` | `RiskGateInputs` (holder concentration, mint authority flags, depth ratios) |
| `sizing::evaluate` | `libraries/engine/src/sizing.rs` | `SizingInput` (capital, hurdle, `l_a` as a plain `f64`) |
| `triggers::evaluate` | `libraries/engine/src/triggers.rs` | `TriggersInput` (history points, fee-jump multiplier) |
| `regime::classify_candidate` | `libraries/engine/src/regime.rs` | volatility scalars, peg deviation, age |

The ranking metric itself -- `r_ratio` / `r_org` in `libraries/dlmm_math/src/ranking.rs` -- is one
expression:

```
R = 2 . f_hat . tau_a . geometry . (1 - protocol_share) / sigma_d^2
```

`f_hat` (forecast fee), `tau_a` (turnover), `protocol_share` and `sigma_d` (daily volatility) mean
the same thing regardless of venue. `geometry` is the one term that is venue-specific, described in
section 2. `y_fee` (expected fee yield at a given position size) has the same property.

### The storage side of "shared"

The migration comment on `indicators_5m` (`migrations/0015_indicators.sql`) states this directly:

> `venue` is added here (and on signals, rationale, paper_positions, outcomes): the ranking
> metrics reduce to one shared expression across venues, so the only thing these tables need to
> grow for a second venue is rows with a different `venue` value.

`indicators_{5m,10m,1h,4h,24h}`, `signals`, `rationale`, `paper_positions`, `outcomes`, and the
read paths that serve them (`libraries/storage/src/queries/top_pools.rs`,
`volume_ranking.rs`, `potential_pools.rs`) all carry a `venue SMALLINT` column and filter on it,
but none of them join back to a venue-specific satellite table. `top_pools_5m` in
`libraries/storage/src/queries/top_pools.rs`, for example, joins only `indicators_5m` to `pools`:
it reads results that are already venue-agnostic by the time they are written. This is the layer
where the "one shared expression" claim is fully realised in the schema, not just the math.

## 2. What differs, and where the code isolates it

### The `Venue` trait

```rust
// libraries/dlmm_math/src/ranking.rs
pub trait Venue: Send + Sync {
    fn id(&self) -> VenueId;
    fn fee_rate(&self, pool: &PoolState, vol: &VolEstimate) -> Result<FeeRate, MathError>;
    fn turnover_base(&self, pool: &PoolState) -> Option<f64>;
    fn lvr_geometry(&self, pool: &PoolState) -> f64;
    fn extra_gates(&self, pool: &PoolState) -> Vec<RationaleItem>;
}
```

Four methods, and the trait's own doc comment names them as the whole seam: "`fee_rate`,
`turnover_base` and `lvr_geometry` are the only venue-specific inputs to ranking; everything
downstream of them is shared." `extra_gates` exists for a venue that needs a rejection rule the
shared risk gate does not express; DLMM's implementation returns an empty vector.

`dlmm_math::PoolState` (not to be confused with `dlmm_decode::PoolState`, the on-chain account
layout -- see the naming note in section 5) is the trait's own minimal input type: bin step, base
fee factor, variable fee control, active-bin liquidity, protocol share. The engine layer's richer
state is reduced to this at the call site in `libraries/engine/src/pipeline.rs`, and the math crate
itself stays free of I/O.

`libraries/dlmm_math/src/ranking.rs` defines exactly one implementation, `Dlmm`:

```rust
impl Venue for Dlmm {
    fn turnover_base(&self, pool: &PoolState) -> Option<f64> {
        if pool.active_bin_liquidity > 0.0 { Some(pool.active_bin_liquidity) } else { None }
    }
    fn lvr_geometry(&self, pool: &PoolState) -> f64 {
        pool.bin_step_bps as f64 / 10_000.0
    }
    // ...
}
```

`turnover_base` is where "active liquidity" is defined per venue -- for DLMM it is `L_a`, the
reserves sitting in the single bin that price currently occupies. `lvr_geometry` is the width term
in the ranking metric's denominator; for DLMM it is the bin step itself, expressed as a fraction.
The trait doc comment on `r_ratio` states the intended generalisation explicitly: writing the
function once against `geometry` means DLMM's ranking metric (`geometry = s`, the bin step) and a
prospective ranged-AMM's version (`geometry = g/2`, half the range width) are "literally the same
expression -- the algebra that shows DAMM v2's `sigma^2 V/(4g)` reduces to DLMM's `sigma^2 V/(2w)`
at narrow ranges." That reduction is asserted in the doc comment, not tested against a second
`Venue` implementation, because no second implementation exists (`VenueId::DammV2` is a bare enum
variant -- see section 5).

`VenueId` and the `venue_smallint` mapping (`libraries/engine/src/indicators.rs`) already
anticipate a second value (`VenueId::DammV2 => 1`), matching `pools.venue`'s encoding, even though
nothing produces that value yet.

### Where the isolation is incomplete

Not every geometry-shaped input goes through `Venue`. `libraries/engine/src/organic_flow.rs`'s
`OrganicFlowInput` takes a `bin_step: f64` field directly, and `dlmm_math::phi_mech`
(`libraries/dlmm_math/src/organic_flow.rs`) uses it as `(sigma_d / bin_step)^2` -- the same kind of
price-crossing-granularity term as `lvr_geometry`, computed the same way, but wired around the
trait rather than through it. In `libraries/engine/src/pipeline.rs`, the caller builds this field
as `input.bin_step_bps as f64 / 10_000.0`, the identical expression `Dlmm::lvr_geometry` computes,
just duplicated rather than reused via `venue.lvr_geometry(&pool)`. A second venue's organic-flow
estimate would silently keep using DLMM's bin step unless this call site is also changed -- the
`Venue` trait does not cover it, so there is nothing to override.

### Layering

```
 venue-agnostic ------------------------------------------------------------
   engine:  volatility, regime, risk_gate, sizing, triggers   (scalars in, scalars out)
   storage: indicators_{tf}, signals, rationale, paper_positions, outcomes
            (venue column, no satellite join)
 -----------------------------------------------------------------------------
 parameterised over Venue ----------------------------------------------------
   engine:  fee_forecast::evaluate<V: Venue>, ranking::evaluate<V: Venue>,
            pipeline::screen<V: Venue> / pipeline::rank<V: Venue>
   dlmm_math: the Venue trait itself, r_ratio / r_org / y_fee
 -----------------------------------------------------------------------------
 venue-specific ---------------------------------------------------------------
   dlmm_math: Dlmm (the only impl), bin_price, base_fee_rate, endogenous_fee_rate
   dlmm_decode: LbPair/BinArray/Position wire layout, event discriminators
   storage: dlmm_pool_params, dlmm_pool_state, bin_states, active_bin_snapshots
 -----------------------------------------------------------------------------
 wired around the seam (see section 5) ----------------------------------------
   engine:  organic_flow::OrganicFlowInput.bin_step (bypasses Venue)
   storage: scoring_universe / pool_detail / watch_set / paper_position_lifecycle
            queries JOIN dlmm_pool_params directly, not gated on venue
```

## 3. A concrete walkthrough for each anticipated venue

None of the following three sections describes anything in the tree. They are read against the
`Venue` trait's shape to say what implementing it would look like, and against general knowledge of
each program's mechanics, which is not verified against this codebase because none of these venues
appear in it at all -- no file under `libraries/` or `bin/` mentions Raydium, Orca, or Whirlpool,
and DAMM v2 appears only in comments as a stated intention (`migrations/0002_pools.sql`,
`migrations/0009_pool_snapshots.sql`, `libraries/dlmm_math/src/ranking.rs`). Where a mechanic below
is stated with confidence, it is public, well-documented protocol behavior; where it is not, this
document says so rather than guessing.

### Meteora DAMM v2

DAMM v2 is Meteora's constant-product-style pool with a single configurable liquidity range per
position (as opposed to DLMM's many discrete bins), and its own dynamic fee scheduler. The
migration comments already reserve its shape:

- **New tables**: `damm_pool_params` (satellite to `pools`, analogous to `dlmm_pool_params`) and a
  `damm_pool_state`-equivalent (satellite to `pool_snapshots`, analogous to `dlmm_pool_state`) --
  named directly in the `0002` and `0009` migration comments as the expected next `CREATE TABLE`.
- **New code**: a `DammV2` struct implementing `Venue` in a new module (most naturally
  `libraries/dlmm_math/src/ranking.rs` alongside `Dlmm`, or a venue-specific math module if the fee
  scheduler's algebra is large enough to want its own file). `lvr_geometry` returns `g/2` where `g`
  is the position's range width, per the reduction the trait's doc comment already asserts.
  `turnover_base` returns the liquidity active within that range rather than a single bin's
  reserves -- conceptually close to DLMM's `L_a`, but sourced from a continuous range rather than a
  discrete bin index. A `damm_decode` crate (or a shared reworking of `dlmm_decode`) would be
  needed for its account and event wire formats, since DAMM v2 is a different on-chain program with
  its own account layout, not a variant of DLMM's.
- **Reused unchanged**: every stage listed as venue-agnostic in section 1, `ranking::evaluate`,
  `fee_forecast::evaluate`, the full `indicators_{tf}`/`signals`/`rationale` storage and read path,
  and the `pools` table itself (a new row with `venue = 1`).
- **Needs thought**: the fee scheduler's shape is not established anywhere in this tree, so
  `fee_rate`'s implementation is unconstrained by anything written down; whatever DAMM v2's own fee
  update rule turns out to be, it plugs into the same `FeeRate { current, forecast }` output. More
  substantially, the query-layer coupling described in section 4 -- `scoring_universe`,
  `pool_detail`, `watch_set`, `paper_position_lifecycle` all currently `JOIN dlmm_pool_params`
  unconditionally -- would need a second query (or a restructured, venue-branching one) before a
  DAMM v2 pool could reach the pipeline at all, regardless of how clean its `Venue` impl is.

### Raydium CLMM

Raydium CLMM is a tick-based concentrated-liquidity design (a Uniswap-v3-style fork): liquidity is
supplied over `[tick_lower, tick_upper]` ranges rather than DLMM's fixed-width bins, price moves
along a `1.0001^tick` ladder analogous in shape to DLMM's `(1 + bin_step)^bin_id` (both are
geometric price ladders; DLMM's ladder step is configurable per pool, a tick's is fixed at 1 bp),
and liquidity is tracked as the `L` invariant (`x*y = L^2` locally) rather than raw per-bin token
reserves.

- **New tables**: a `raydium_clmm_pool_params` satellite (tick spacing, fee tier, the program's own
  protocol-fee split) and a state satellite tracking current tick and the active liquidity `L`
  at that tick, mirroring `dlmm_pool_state`'s split of "what changes every update" from "what is
  set at pool creation."
- **New code**: a `raydium_decode` crate for its account layout (this program's accounts are
  publicly documented but not vendored anywhere in this repository, so the exact struct layout is
  not something this document can state precisely without reading Raydium's own IDL) and a
  `Venue` impl whose `lvr_geometry` is a tick-range-width term structurally like DAMM v2's `g/2`,
  and whose `turnover_base` is liquidity at the currently active tick -- the closest direct analogue
  to DLMM's `L_a` among the three anticipated venues, since ticks are, like bins, discrete
  price points with liquidity concentrated at whichever one price currently sits in.
- **Reused unchanged**: the same list as DAMM v2 -- every venue-agnostic engine stage and the full
  indicator/signal storage path.
- **Needs thought**: `active_bin_snapshots` and `bin_states` (`migrations/0007`, `migrations/0008`)
  are DLMM-specific tables tied to DLMM's bin indexing; a tick-based venue's equivalent
  high-frequency and full-distribution tables would index by tick rather than bin, which is a
  straightforward rename in spirit but a different column (`tick_id` vs `bin_id`) and likely a
  different natural chunk size, since tick ranges can be far wider than DLMM's typical ~210-bin
  window mentioned in `migrations/0008_bin_states.sql`. Fee accrual on a tick-based CLMM is
  typically tracked as a global fee-growth accumulator checked against tick boundaries, which is
  structurally similar to DLMM's `fee_x_per_token_stored` (also a monotonic accumulator, per
  `migrations/0008`'s comment) but the exact differencing logic against tick crossings is not
  something this document verifies from source, since no such source exists here.

### Orca Whirlpool

Orca Whirlpool is also a tick-based concentrated-liquidity AMM, structurally close to Raydium CLMM
(both are commonly described as Uniswap-v3-derived designs), with its own account layout and fee
tier structure. Everything said above about Raydium CLMM's relationship to the `Venue` trait --
tick-range geometry, active-liquidity-at-current-tick as the turnover base, a tick-indexed
equivalent of `bin_states`/`active_bin_snapshots` -- applies in the same shape here. The specific
differences between Whirlpool's and Raydium CLMM's account layouts and fee mechanics are not
something this document can respond to precisely: neither appears anywhere in this codebase, and
the two programs are similar enough in outline (tick ladder, per-tick liquidity, a global fee-growth
accumulator) that stating a difference with confidence here would be guessing. Concretely, adding
Whirlpool after Raydium CLMM was already added would mean: a third `Venue` impl, a third decode
crate, and a third pair of satellite tables -- but whether its `Venue` impl can share code with
Raydium CLMM's beyond the trait boundary is a question this document leaves open rather than
answers.

## 4. The database story

The claim, stated directly in the first migration's own comment
(`migrations/0002_pools.sql`):

> Class-table inheritance: `pools` carries what every venue has, a satellite table per venue
> carries what only that venue has. Adding a second venue is a new satellite table plus new
> `pools` rows with a different `venue` value -- never an `ALTER` of a populated table.

This is verified, not just asserted:

- `pools.venue` is `SMALLINT NOT NULL` with **no `CHECK` constraint**. The column comment states
  the reason directly: "a future venue is a new value, not a schema change, and a `CHECK` here
  would force an `ALTER` on a populated table the day a second venue is added." A grep across
  `migrations/` confirms there is no `CHECK` on `venue` anywhere, and `libraries/storage/src/types.rs`
  keeps the venue encoding as a plain `pub const DLMM: i16 = 0;` in a `pub mod venue`, not a Rust
  enum backed by a database enum type -- the same reasoning applied one layer up: "a new venue is
  a new value, not a schema change."
- Every venue-specific column lives on a satellite table (`dlmm_pool_params`,
  `dlmm_pool_state`) that references `pools(pool_address)`, never on `pools` itself.
  `bin_states` and `active_bin_snapshots` are DLMM-only tables with no generic counterpart; a
  second venue with a different high-frequency state shape gets its own pair, not a shared one
  (section 5 covers what "different shape" could mean for a venue with no per-bin state at all).
- `indicators_{tf}`, `signals`, `rationale`, `paper_positions`, `outcomes` all carry `venue`
  directly and require no satellite join to be read (section 1). Adding a second venue's rows to
  these tables is purely additive by construction, since they were designed as flat, venue-tagged
  tables from the start.

So the schema-level claim holds: nothing in `migrations/` would need an `ALTER` on a populated
table to add a venue. What the schema alone does not show is that four query functions in
`libraries/storage/src/queries/` currently defeat that additivity at the read layer by hardcoding
the DLMM satellite table into their `JOIN`:

| Query | File | Line |
|---|---|---|
| `scoring_universe` | `libraries/storage/src/queries/scoring_universe.rs` | `JOIN dlmm_pool_params d ON d.pool_address = p.pool_address` |
| `pool_detail` | `libraries/storage/src/queries/pool_detail.rs` | `JOIN dlmm_pool_params d ON d.pool_address = p.pool_address` |
| `watch_set` | `libraries/storage/src/queries/watch_set.rs` | `JOIN dlmm_pool_params d ON d.pool_address = p.pool_address` |
| `paper_position_lifecycle` | `libraries/storage/src/queries/paper_position_lifecycle.rs` | `JOIN dlmm_pool_params d ON d.pool_address = pp.pool_address` |
| `rollup_source` | `libraries/storage/src/queries/rollup_source.rs` | `LEFT JOIN dlmm_pool_state d ON d.pool_address = s.pool_address AND d.ts = s.ts` |

`scoring_universe` takes `venue: i16` as a parameter and filters `WHERE p.venue = $1`, which reads
as venue-generic at the call signature -- but its inner `JOIN dlmm_pool_params` means it returns
zero rows for any venue whose pools have no row in that table, which is every venue but DLMM. This
is the single largest gap between what the schema promises and what the query layer currently
delivers: the tables are additive, but the query that assembles a pool's parameters for scoring is
not, and would need a venue-conditional rewrite (or a parallel query per venue) before a second
venue's pools could reach the pipeline. `rollup_source`'s join is a `LEFT JOIN`, which is more
forgiving -- a pool with no `dlmm_pool_state` row simply gets `NULL`s there rather than being
dropped -- but it is still a query that only knows about one satellite table by name.

## 5. Where the abstraction would strain

Every seam above has a limit. Listed from least to most structural:

**The query layer is the real bottleneck, not the `Venue` trait.** Section 4's table of hardcoded
joins is the most concrete, fixable gap: the math generalises, the schema generalises, and the
piece connecting them does not, today. This is the first thing that would need to change, before
any `Venue` implementation work, to onboard a second venue.

**`organic_flow` bypasses `Venue` entirely.** As section 2 describes, `phi_mech`'s bin-step term is
wired directly from `PipelineInput.bin_step_bps` in `libraries/engine/src/pipeline.rs`, duplicating
the expression inside `Dlmm::lvr_geometry` rather than calling it. Nothing enforces that a second
venue's geometry stays consistent between its ranking metric and its organic-flow estimate --
that consistency exists today only because there is one venue and one geometry value.

**`VenueId::DammV2` is a name, not an implementation.** The enum variant, the `venue_smallint`
mapping, and the doc comments describing DAMM v2's algebra all exist. No `struct DammV2`, no
`impl Venue for DammV2`, and no test analogous to
`test_rank_via_venue_trait_matches_worked_example_a` (`libraries/dlmm_math/src/ranking.rs`) exists
to confirm the asserted `sigma^2 V/(4g)` -> `sigma^2 V/(2w)` reduction actually holds in code. The
trait's design is a real seam; whether a second venue slots into it without surprises is untested,
because there is nothing to test it against yet.

**`Source`'s payload types are DLMM-shaped, not venue-generic.** `Source::state_stream` returns
`StateUpdate` (`libraries/source/src/domain.rs`), whose fields are `lb_pair: Option<PoolState>` and
`bin_arrays: Vec<BinArrayState>` -- both imported directly from `dlmm_decode`
(`use dlmm_decode::{BinArrayState, DecodedEvent, PoolState};`). `Source` is generic over the
*ingestion mechanism* (RPC vs. Geyser both implement it identically), but its concrete payload is
hardwired to one venue's decoded account shapes. A second venue cannot flow through the existing
`Source` trait as written -- either a second `Source`-shaped trait per venue, or a redesign of
`StateUpdate`/`ChainEvent` into a venue-polymorphic payload (an enum over decoded types, most
plausibly) would be needed. This is a larger change than adding a `Venue` impl, because it touches
both `bin/indexer` workers that consume `StateUpdate` today.

**Every running binary hardcodes `venue::DLMM` at the call site.** `bin/indexer/src/workers/discovery.rs`
and `bin/indexer/src/workers/tier.rs` call `scoring_universe`, `top_pools`, and related functions
with the `venue::DLMM` constant (`storage::types::venue::DLMM`) as a literal argument, not a
configured or discovered value.
The storage functions themselves are venue-parameterized; the binaries that call them are not.
Multi-venue *operation* -- one running system scoring pools across two venues at once -- does not
exist today even where the underlying function signatures would allow it.

**`bin_states`/`active_bin_snapshots` assume there is such a thing as "a bin."** Both tables are
indexed by `bin_id`. A tick-based CLMM (Raydium, Orca) has a direct analogue (`tick_id`), so this
strains only at the level of a rename plus a different natural granularity, as section 3 notes. A
venue with genuinely no discretized liquidity structure -- a plain constant-product pool, where
liquidity is smeared continuously across the entire price range rather than concentrated anywhere
-- has no equivalent at all. None of the three anticipated venues are constant-product (DAMM v2,
despite older Meteora constant-product pools existing under a different program, is itself a
concentrated-range design per its migration-comment description), so this is out of scope for what
is actually planned, but it marks where the abstraction's premise stops applying: `turnover_base`
and `lvr_geometry` are built around the idea that some liquidity is "active" and some is not. A
constant-product pool's liquidity is uniformly active by construction, which does not make the
trait's methods wrong so much as trivial (`turnover_base` would just be TVL, `lvr_geometry` would
be a constant), and the entire framing of `r_ratio` as a fee/LVR ratio *at the active bin* stops
correlating with anything distinguishing between pools of the same venue, since every pool of that
venue would score on total TVL alone. At that point the ranking metric is not so much generalising
as degenerating.

**Fee accrual schedule is assumed, not abstracted.** `dlmm_pool_state.total_fee_bps` and
`bin_states.fee_x_per_token_stored`/`fee_y_per_token_stored` are, per the `0008` migration
comment, monotonically non-decreasing per-token accumulators, which is why the system can compute
accrual between two 5-minute polls as a plain difference of endpoints. This is true of DLMM and,
by the general shape of fee-growth accumulators, likely true of tick-based CLMMs too (Raydium,
Orca). It is not guaranteed to be true of every venue in general -- a venue that compounds fees
directly into position liquidity rather than tracking them separately, or that distributes fees on
an epoch/reward-cycle basis rather than continuously, would break the "difference of two endpoints"
computation this system relies on throughout its snapshot tables. Nothing in the code detects or
guards against this; it is an assumption baked into the schema design, not a case the `Venue` trait
covers.

**"Venue" is an overloaded word in this codebase, worth flagging for a reader.** `risk_gate.rs`'s
`other_venue_min_depth_ratio` and `other_venue_depth_ratio` (`libraries/engine/src/risk_gate.rs`)
refer to depth on a *competing* market -- another DEX, or a CEX listing -- as a risk signal. That
has nothing to do with `pools.venue` or the `Venue` trait; it is an unrelated use of the same
English word for a different concept. Both readings coexist in the source and are easy to conflate
on a quick read.

## Summary

The ranking math generalises the way the codebase's comments say it does: `r_ratio`/`r_org`/`y_fee`
are written once, the `Venue` trait names exactly three venue-specific inputs, and the schema is
additive by actual construction (no `CHECK`, no enum type, satellite tables only). That part of the
claim is stronger, not weaker, than a skeptical read would expect going in.

What does not yet match the ambition: the query layer between storage and the pipeline is
hardwired to DLMM's satellite tables in five places, one pipeline stage (`organic_flow`) reads
DLMM's geometry around the trait rather than through it, the ingestion trait's payload type is
DLMM-shaped, and no second `Venue` implementation exists to prove out the trait's design under
real use. Adding Meteora DAMM v2 -- the closest anticipated venue, since it shares Meteora's own
program family and the migration comments already name its tables -- is the cheapest of the three
to add, and would surface all of these gaps in roughly the order listed in section 5. Raydium CLMM
and Orca Whirlpool would additionally require their own decode crates from scratch, and their
account layouts are not something this document can respond to without reading each program's IDL
directly, since neither appears anywhere in this repository today.
