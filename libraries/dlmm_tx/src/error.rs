/// Errors this crate can return while building an instruction. Every variant catches a
/// condition the DLMM program would otherwise reject at runtime with an opaque Anchor error
/// code -- catching it here gives the Telegram Mini App a message it can show the user before
/// ever asking them to sign.
#[derive(Debug, thiserror::Error)]
pub enum DlmmTxError {
    #[error("bin range is inverted: from {from} is greater than to {to}")]
    InvertedBinRange { from: i32, to: i32 },

    #[error("amount must be greater than zero")]
    ZeroAmount,

    #[error(
        "position width {width} is out of range: must be between 1 and {max} bins \
         (a wider position needs increase_position_length after opening)"
    )]
    WidthOutOfRange { width: i32, max: i32 },

    #[error("bps_to_remove {bps} is out of range: must be between 1 and 10000")]
    BpsOutOfRange { bps: u16 },

    #[error("bin range [{from}, {to}] falls outside the position's own range [{lower}, {upper}]")]
    RangeExceedsPosition {
        from: i32,
        to: i32,
        lower: i32,
        upper: i32,
    },
}
