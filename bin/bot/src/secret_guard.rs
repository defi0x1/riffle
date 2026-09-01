// The one place a real user is most likely to hurt themselves: pasting a private key or seed
// phrase into `/wallet`, thinking that is how a wallet gets "connected", when the design is
// keyless end to end and `/wallet` only ever accepts a public key. This has to be caught on the
// raw, untokenized message text, before clap ever sees it -- a seed phrase is many
// whitespace-separated tokens, and clap's own "unexpected argument" error text echoes the
// offending token back, which would put a fragment of the key into the chat a second time.
//
// Detection is structural only (see `shape`): length and base58/BIP-39-word shape, never an
// attempt to decode or validate the value as a real key. That keeps this check honest about
// what it can promise -- it catches the shapes a real key or phrase actually takes, not "is
// this cryptographically a key".
use crate::cli::normalize_command_token;
use crate::shape::{looks_like_raw_secret_key, looks_like_secret_key_array, looks_like_seed_phrase};

pub fn wallet_message_carries_key_material(text: &str) -> bool {
    let mut tokens = text.split_whitespace();
    let Some(first) = tokens.next() else {
        return false;
    };
    if normalize_command_token(first) != "wallet" {
        return false;
    }

    let rest: Vec<&str> = tokens.collect();
    if looks_like_seed_phrase(&rest) {
        return true;
    }
    rest.iter()
        .any(|t| looks_like_raw_secret_key(t) || looks_like_secret_key_array(t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flags_a_seed_phrase_after_wallet() {
        let text = "/wallet abandon ability able about above absent absorb abstract absurd abuse access accident";
        assert!(wallet_message_carries_key_material(text));
    }

    #[test]
    fn test_flags_a_raw_base58_secret_key_after_wallet() {
        let text = format!("/wallet {}", "A".repeat(87));
        assert!(wallet_message_carries_key_material(&text));
    }

    #[test]
    fn test_flags_a_keypair_array_after_wallet() {
        let numbers: Vec<String> = (0..64u16).map(|n| n.to_string()).collect();
        let text = format!("/wallet [{}]", numbers.join(","));
        assert!(wallet_message_carries_key_material(&text));
    }

    #[test]
    fn test_does_not_flag_a_legitimate_registration() {
        let text = "/wallet 7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU main";
        assert!(!wallet_message_carries_key_material(text));
    }

    #[test]
    fn test_does_not_flag_a_bare_wallet_list_request() {
        assert!(!wallet_message_carries_key_material("/wallet"));
    }

    #[test]
    fn test_only_applies_to_the_wallet_command() {
        // Twelve short words after an unrelated command is not this check's concern -- worker
        // never routes non-slash or other-command text through it either.
        let text = "/pool abandon ability able about above absent absorb abstract absurd abuse access accident";
        assert!(!wallet_message_carries_key_material(text));
    }

    #[test]
    fn test_strips_group_mention_suffix_before_checking() {
        let text = format!("/wallet@FeeFarmBot {}", "A".repeat(87));
        assert!(wallet_message_carries_key_material(&text));
    }
}
