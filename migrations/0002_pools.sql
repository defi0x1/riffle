-- Class-table inheritance: `pools` carries what every venue has, a satellite
-- table per venue carries what only that venue has. Adding a second venue is
-- a new satellite table plus new `pools` rows with a different `venue` value
-- never an ALTER of a populated table. This is the one seam that is
-- expensive to retrofit, so it is drawn on the first migration.

CREATE TABLE pools (
    pool_address        TEXT PRIMARY KEY,
    -- 0 = DLMM today. No CHECK constraint: a future venue is a new value,
    -- not a schema change, and a CHECK here would force an ALTER on a
    -- populated table the day a second venue is added.
    venue               SMALLINT NOT NULL,
    token_x             TEXT NOT NULL,
    token_y             TEXT NOT NULL,
    -- Normalised current base fee rate. Every venue has one, even though how
    -- it is derived differs (DLMM: bin_step x base_factor; DAMM v2: its own
    -- fee scheduler). The venue-specific inputs live in the satellite table;
    -- this column is the comparable, already-reduced number.
    base_fee_bps        NUMERIC(20,6) NOT NULL,
    -- Read per pool, never assumed constant -- it varies pool to pool on
    -- live data even within one venue.
    protocol_share_bps  INTEGER NOT NULL,
    -- Denormalised cache of the latest pool_snapshots.tvl_usd, refreshed on
    -- every snapshot write. Lets the ~124k-pool screening queries (/top,
    -- /volume) run without touching the pool_snapshots hypertable.
    tvl_usd              NUMERIC(38,18),
    status               SMALLINT NOT NULL,
    -- 0 = universe (screened only), 1 = watched (bin state subscribed,
    -- quality-A indicators available). See indicators_{tf}.quality.
    tier                 SMALLINT NOT NULL DEFAULT 0,
    tier_changed_at      TIMESTAMPTZ,
    creator              TEXT,
    activation_point     BIGINT,
    created_at           TIMESTAMPTZ NOT NULL,
    -- Age gate for the V2 regime; NULL until the pool has ever held liquidity.
    first_liquidity_at   TIMESTAMPTZ,
    is_blacklisted        BOOLEAN NOT NULL DEFAULT FALSE,
    launchpad             TEXT,
    tags                  TEXT[] NOT NULL DEFAULT '{}',
    updated_at             TIMESTAMPTZ NOT NULL
);

-- No FK from pools to tokens(mint): a pool is frequently discovered before
-- its mints' metadata has been fetched, and forcing insert order here would
-- couple pool discovery to the token-metadata fetch path for no benefit.
CREATE INDEX idx_pools_venue_tier ON pools (venue, tier);
CREATE INDEX idx_pools_tier_tvl ON pools (tier, tvl_usd DESC);

-- DLMM-only parameters. StaticParameters plus the fields that drive the
-- dynamic fee. A second venue (e.g. DAMM v2) gets its own
-- `damm_pool_params` table later -- a pure CREATE TABLE.
CREATE TABLE dlmm_pool_params (
    pool_address                TEXT PRIMARY KEY REFERENCES pools (pool_address),
    bin_step                    SMALLINT NOT NULL,
    base_factor                 INTEGER NOT NULL,
    filter_period                INTEGER NOT NULL,
    decay_period                 INTEGER NOT NULL,
    reduction_factor              INTEGER NOT NULL,
    variable_fee_control          INTEGER NOT NULL,
    max_volatility_accumulator     INTEGER NOT NULL,
    collect_fee_mode             SMALLINT NOT NULL,
    -- Farm rewarder mints, if any. DLMM-specific account structure; not a
    -- concept every venue shares, so it does not belong on `pools`.
    reward_mint_x                TEXT,
    reward_mint_y                TEXT
);
