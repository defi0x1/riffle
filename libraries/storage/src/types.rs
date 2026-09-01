// Shared constants and small enums used across write and read modules. Deliberately not a
// wrapper type per column: `pools.venue`, `.tier` and `indicators_{tf}.quality` are stored as
// their raw SQL types (SMALLINT, CHAR(1)) everywhere in this crate, matching how the migrations
// themselves document the encoding in a comment rather than a CHECK constraint -- a new venue is
// a new value, not a schema change.

pub mod venue {
    pub const DLMM: i16 = 0;
}

pub mod tier {
    pub const UNIVERSE: i16 = 0;
    pub const WATCHED: i16 = 1;
}

pub mod quality {
    pub const MEASURED: &str = "A";
    pub const ESTIMATED: &str = "B";
}

pub mod liquidity_action {
    pub const ADD: i16 = 0;
    pub const REMOVE: i16 = 1;
}

// A user action that becomes an unsigned transaction. Closed set enforced by a CHECK constraint
// on transaction_intents.action (see 0030) -- unlike `venue`, a new kind of on-chain action is
// rare enough, and consequential enough to get wrong, that a migration is the right cost to pay.
pub mod intent_action {
    pub const OPEN: i16 = 0;
    pub const ADD: i16 = 1;
    pub const REMOVE: i16 = 2;
    pub const CLAIM: i16 = 3;
    pub const CLOSE: i16 = 4;
}

// transaction_intents.status. CREATED -> SUBMITTED -> {CONFIRMED | FAILED}, or CREATED /
// SUBMITTED -> EXPIRED if the client never comes back. CONFIRMED is terminal and is never
// overwritten by a later FAILED or EXPIRED write -- see confirm_transaction_intent /
// mark_intent_failed.
pub mod intent_status {
    pub const CREATED: i16 = 0;
    pub const SUBMITTED: i16 = 1;
    pub const CONFIRMED: i16 = 2;
    pub const FAILED: i16 = 3;
    pub const EXPIRED: i16 = 4;
}

// position_cash_flows.kind: which direction value moved across the position boundary.
pub mod cash_flow_kind {
    pub const DEPOSIT: i16 = 0;
    pub const WITHDRAWAL: i16 = 1;
    pub const FEE_CLAIM: i16 = 2;
}

// The five rollup/derived timeframes. Used to dispatch to the literal per-table query a given
// timeframe needs -- sqlx::query!/query_as! require a literal table name, so a table name cannot
// be parameterised at the SQL level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Timeframe {
    M5,
    M10,
    H1,
    H4,
    H24,
}

impl Timeframe {
    pub fn as_str(self) -> &'static str {
        match self {
            Timeframe::M5 => "5m",
            Timeframe::M10 => "10m",
            Timeframe::H1 => "1h",
            Timeframe::H4 => "4h",
            Timeframe::H24 => "24h",
        }
    }

    pub const ALL: [Timeframe; 5] = [
        Timeframe::M5,
        Timeframe::M10,
        Timeframe::H1,
        Timeframe::H4,
        Timeframe::H24,
    ];
}
