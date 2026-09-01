#[derive(Debug, thiserror::Error)]
pub enum MathError {
    #[error("lb_clmm price/fee computation overflowed")]
    Overflow,
}
