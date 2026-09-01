use std::sync::LazyLock;

use solana_sdk::hash::hash;

// Anchor discriminators are sha256("<namespace>:<name>")[..8], computed at IDL-generation time
// and baked into the program. "account" for account structs, "event" for #[event] structs. This
// mirrors anchor-lang's own `sighash` (anchor-attribute-{account,event}), not something copied
// from lb_clmm itself -- lb_clmm never spells out its own discriminators, they fall out of the
// macro. Computing them here means a renamed or added event/account in a future lb_clmm release
// is handled by just changing the name string, not a byte array nobody can explain.
pub fn discriminator(namespace: &str, name: &str) -> [u8; 8] {
    let digest = hash(format!("{namespace}:{name}").as_bytes()).to_bytes();
    let mut out = [0u8; 8];
    out.copy_from_slice(&digest[..8]);
    out
}

// The fixed 8-byte prefix Anchor's `emit_cpi!` puts in front of every self-CPI event
// instruction, ahead of the per-event discriminator. It is the same for every event of every
// Anchor program: anchor_lang::event::EVENT_IX_TAG_LE. Unlike account/event discriminators
// (raw sha256 prefix bytes, see `discriminator` above), anchor-lang builds this one as a u64
// constant from those same prefix bytes and then calls `.to_le_bytes` on it, which reverses
// the byte order relative to the sha256 digest -- a one-off encoding quirk of that single
// constant, not the general discriminator scheme.
pub static EVENT_IX_TAG: LazyLock<[u8; 8]> = LazyLock::new(|| {
    let mut tag = discriminator("anchor", "event");
    tag.reverse();
    tag
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_ix_tag_matches_anchor_lang_constant() {
        // anchor_lang::event::EVENT_IX_TAG_LE, reproduced here byte-for-byte to confirm our
        // sha256-prefix derivation lines up with anchor-lang's own hardcoded constant.
        assert_eq!(
            *EVENT_IX_TAG,
            [0xe4, 0x45, 0xa5, 0x2e, 0x51, 0xcb, 0x9a, 0x1d]
        );
    }

    #[test]
    fn test_discriminator_is_stable_and_namespaced() {
        assert_eq!(
            discriminator("account", "LbPair"),
            discriminator("account", "LbPair")
        );
        assert_ne!(
            discriminator("account", "LbPair"),
            discriminator("event", "LbPair")
        );
    }
}
