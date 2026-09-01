#[derive(Debug, thiserror::Error)]
pub enum MathError {
    #[error("DLMM price/fee computation overflowed")]
    Overflow,
}
