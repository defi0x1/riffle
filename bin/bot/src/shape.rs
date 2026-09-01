// Structural checks over raw strings -- shape only, never a cryptographic or wordlist
// validation. Two callers need these: `/wallet` registration rejects a value that plainly is
// not pubkey-shaped before it ever reaches storage, and `secret_guard` uses the same shape
// tests to catch a private key or seed phrase pasted where a public key belongs -- deliberately
// without ever trying to decode or validate the value as a real key, since doing that would
// mean handling it as more than an opaque string for longer than necessary.

const BASE58_ALPHABET: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

fn is_base58(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| BASE58_ALPHABET.contains(c))
}

// A Solana public key, base58-encoded, is always 32-44 characters: 32 raw bytes, shortened by
// any leading zero bytes. Used to keep `/wallet` from storing obvious garbage as a "registered
// pubkey" -- not a substitute for the chain itself being the real authority on whether a key
// exists.
pub fn looks_like_pubkey(s: &str) -> bool {
    let len = s.chars().count();
    (32..=44).contains(&len) && is_base58(s)
}

// A raw 64-byte Solana secret key, base58-encoded, is 87-88 characters -- comfortably past any
// real pubkey's range. The floor is set well below that so a truncated or partially-redacted
// paste is still caught.
pub fn looks_like_raw_secret_key(s: &str) -> bool {
    let len = s.chars().count();
    len >= 50 && is_base58(s)
}

// A Solana CLI keypair file is a JSON array of 64 small integers. Not base58 at all, so it
// needs its own shape test rather than falling out of the one above.
pub fn looks_like_secret_key_array(s: &str) -> bool {
    let trimmed = s.trim();
    let Some(inner) = trimmed
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    else {
        return false;
    };
    let entries = inner.split(',').filter(|e| !e.trim().is_empty()).count();
    entries >= 32 && inner.split(',').all(|e| e.trim().parse::<u16>().is_ok())
}

// BIP-39 mnemonics come in exactly these lengths.
const BIP39_LENGTHS: [usize; 5] = [12, 15, 18, 21, 24];

// Word shape (lowercase ascii letters, 3-8 characters) matches the published wordlist's own
// bounds without checking membership in it -- a fake, foreign-language, or slightly misspelled
// seed phrase is exactly as radioactive as a real one, so shape is the right test, not
// validity.
pub fn looks_like_seed_phrase(words: &[&str]) -> bool {
    if !BIP39_LENGTHS.contains(&words.len()) {
        return false;
    }
    words.iter().all(|w| {
        let len = w.chars().count();
        (3..=8).contains(&len) && w.chars().all(|c| c.is_ascii_alphabetic())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recognizes_a_realistic_pubkey() {
        assert!(looks_like_pubkey(
            "7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU"
        ));
    }

    #[test]
    fn test_rejects_a_short_string_as_pubkey() {
        assert!(!looks_like_pubkey("hello"));
    }

    #[test]
    fn test_rejects_non_base58_characters_as_pubkey() {
        // Contains '0', 'O', 'I', 'l' -- all excluded from the base58 alphabet.
        assert!(!looks_like_pubkey("0OIl000000000000000000000000000"));
    }

    #[test]
    fn test_recognizes_an_87_char_base58_string_as_a_raw_secret_key() {
        let key = "A".repeat(87);
        assert!(looks_like_raw_secret_key(&key));
    }

    #[test]
    fn test_does_not_flag_a_real_pubkey_length_as_a_secret_key() {
        assert!(!looks_like_raw_secret_key(
            "7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU"
        ));
    }

    #[test]
    fn test_recognizes_a_keypair_file_array() {
        let numbers: Vec<String> = (0..64u16).map(|n| n.to_string()).collect();
        let array = format!("[{}]", numbers.join(","));
        assert!(looks_like_secret_key_array(&array));
    }

    #[test]
    fn test_does_not_flag_a_short_array_as_a_keypair() {
        assert!(!looks_like_secret_key_array("[1,2,3]"));
    }

    #[test]
    fn test_recognizes_a_twelve_word_seed_phrase() {
        let words = [
            "abandon", "ability", "able", "about", "above", "absent", "absorb", "abstract",
            "absurd", "abuse", "access", "accident",
        ];
        assert!(looks_like_seed_phrase(&words));
    }

    #[test]
    fn test_does_not_flag_an_arbitrary_sentence_as_a_seed_phrase() {
        let words = ["please", "register", "my", "wallet", "now"];
        assert!(!looks_like_seed_phrase(&words));
    }

    #[test]
    fn test_does_not_flag_a_pubkey_plus_label_as_a_seed_phrase() {
        let words = ["7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU", "main"];
        assert!(!looks_like_seed_phrase(&words));
    }
}
