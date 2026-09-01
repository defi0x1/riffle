//! Server-side verification of Telegram Mini App `initData`, per Telegram's published algorithm
//! (https://core.telegram.org/bots/webapps#validating-data-received-via-the-mini-app). This is
//! the entire authentication boundary for the service: the miniapp's own README states plainly
//! that it never validates `initData` locally and forwards it opaquely as a header on every
//! request, trusting the backend's recomputation of the HMAC as the real check. A caller must
//! never take a Telegram user id from anywhere else -- not a request body field, not a query
//! parameter -- only from this module's return value.
//!
//! `initData` carries no single-use nonce by Telegram's own design: the same string is valid,
//! and meant to be reused, for every request in a Mini App session. `auth_date` recency is the
//! sole defense against a captured header being replayed after the fact, matching what the
//! miniapp's README assumes of the backend.

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TelegramUser {
    pub id: i64,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum InitDataError {
    #[error("missing X-Telegram-Init-Data header")]
    Missing,
    #[error("initData is missing its hash field")]
    MissingHash,
    #[error("initData hash is not valid hex")]
    InvalidHashEncoding,
    #[error("initData hash does not match the computed HMAC")]
    HashMismatch,
    #[error("initData is missing auth_date")]
    MissingAuthDate,
    #[error("initData auth_date is not a valid integer")]
    InvalidAuthDate,
    #[error("initData auth_date is too old")]
    Expired,
    #[error("initData is missing a user field")]
    MissingUser,
    #[error("initData user field is not valid JSON")]
    InvalidUser,
    #[error("initData user JSON has no numeric id")]
    MissingUserId,
}

/// Verifies `raw` against `bot_token`, checking `auth_date` freshness relative to `now`.
/// `now` is threaded through explicitly rather than read from the clock internally so tests can
/// exercise expiry deterministically.
pub fn verify_init_data(
    raw: &str,
    bot_token: &str,
    max_age: Duration,
    now: DateTime<Utc>,
) -> Result<TelegramUser, InitDataError> {
    if raw.is_empty() {
        return Err(InitDataError::Missing);
    }

    let mut pairs: BTreeMap<String, String> = BTreeMap::new();
    for segment in raw.split('&') {
        if segment.is_empty() {
            continue;
        }
        let (key, value) = segment.split_once('=').unwrap_or((segment, ""));
        pairs.insert(percent_decode(key), percent_decode(value));
    }

    let hash_hex = pairs.remove("hash").ok_or(InitDataError::MissingHash)?;
    let expected_mac = decode_hex(&hash_hex).ok_or(InitDataError::InvalidHashEncoding)?;

    // Every remaining pair, sorted by key (BTreeMap already keeps them sorted), joined as
    // "key=value" lines -- Telegram's own data_check_string construction.
    let data_check_string = pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("\n");

    // secret_key = HMAC-SHA256(key = "WebAppData", message = bot_token); the payload's own
    // hash = HMAC-SHA256(key = secret_key, message = data_check_string).
    let mut secret_mac =
        Hmac::<Sha256>::new_from_slice(b"WebAppData").expect("HMAC accepts a key of any length");
    secret_mac.update(bot_token.as_bytes());
    let secret_key = secret_mac.finalize().into_bytes();

    let mut mac =
        Hmac::<Sha256>::new_from_slice(&secret_key).expect("HMAC accepts a key of any length");
    mac.update(data_check_string.as_bytes());
    // `Mac::verify_slice` compares in constant time (via the digest crate's `CtOutput`,
    // itself built on `subtle::ConstantTimeEq`) -- deliberately not a manual byte-by-byte `==`.
    mac.verify_slice(&expected_mac)
        .map_err(|_| InitDataError::HashMismatch)?;

    let auth_date_raw = pairs
        .get("auth_date")
        .ok_or(InitDataError::MissingAuthDate)?;
    let auth_date_secs: i64 = auth_date_raw
        .parse()
        .map_err(|_| InitDataError::InvalidAuthDate)?;
    let auth_date =
        DateTime::<Utc>::from_timestamp(auth_date_secs, 0).ok_or(InitDataError::InvalidAuthDate)?;

    let max_age = chrono::Duration::from_std(max_age).unwrap_or_else(|_| chrono::Duration::days(3650));
    // A small allowance for clock skew in the "payload claims to be from the future" direction
    // -- auth_date is set by Telegram's own servers, not the client, so it is not an
    // attacker-controlled value trying to buy more validity time.
    let skew_allowance = chrono::Duration::seconds(60);
    if now.signed_duration_since(auth_date) > max_age
        || auth_date.signed_duration_since(now) > skew_allowance
    {
        return Err(InitDataError::Expired);
    }

    let user_raw = pairs.get("user").ok_or(InitDataError::MissingUser)?;
    let user_json: serde_json::Value =
        serde_json::from_str(user_raw).map_err(|_| InitDataError::InvalidUser)?;
    let id = user_json
        .get("id")
        .and_then(|v| v.as_i64())
        .ok_or(InitDataError::MissingUserId)?;

    Ok(TelegramUser { id })
}

fn percent_decode(s: &str) -> String {
    // Telegram's own client encodes initData with standard percent-encoding
    // (`encodeURIComponent`), not the HTML-form `+`-for-space convention -- `percent_decode_str`
    // leaves a literal `+` alone, which is the correct behaviour here.
    percent_encoding::percent_decode_str(s)
        .decode_utf8_lossy()
        .into_owned()
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOT_TOKEN: &str = "123456:ABC-test-token-not-real";

    /// Builds a syntactically valid initData string signed with `BOT_TOKEN`, so tests can
    /// exercise the parser and the HMAC check independently of Telegram's own client.
    fn signed_init_data(user_id: i64, auth_date: i64) -> String {
        let user = format!(r#"{{"id":{user_id},"first_name":"Test"}}"#);
        let user_encoded = percent_encoding::utf8_percent_encode(
            &user,
            percent_encoding::NON_ALPHANUMERIC,
        )
        .to_string();

        let mut pairs: BTreeMap<String, String> = BTreeMap::new();
        pairs.insert("auth_date".to_string(), auth_date.to_string());
        pairs.insert("query_id".to_string(), "AAEmock".to_string());
        pairs.insert("user".to_string(), user.clone());

        let data_check_string = pairs
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("\n");

        let mut secret_mac = Hmac::<Sha256>::new_from_slice(b"WebAppData").unwrap();
        secret_mac.update(BOT_TOKEN.as_bytes());
        let secret_key = secret_mac.finalize().into_bytes();

        let mut mac = Hmac::<Sha256>::new_from_slice(&secret_key).unwrap();
        mac.update(data_check_string.as_bytes());
        let hash = hex_encode(&mac.finalize().into_bytes());

        format!(
            "auth_date={auth_date}&query_id=AAEmock&user={user_encoded}&hash={hash}"
        )
    }

    fn hex_encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn test_valid_payload_is_accepted_and_yields_the_telegram_user_id() {
        let now = Utc::now();
        let raw = signed_init_data(42, now.timestamp());
        let user = verify_init_data(&raw, BOT_TOKEN, Duration::from_secs(86_400), now).unwrap();
        assert_eq!(user.id, 42);
    }

    #[test]
    fn test_forged_hash_is_rejected() {
        let now = Utc::now();
        let mut raw = signed_init_data(42, now.timestamp());
        // Flip one hex character in the hash -- a forged or corrupted signature.
        let flipped = raw.replace("hash=a", "hash=b").replace("hash=0", "hash=1");
        if flipped == raw {
            raw.push('0'); // guarantee the string actually changed regardless of hash content
        } else {
            raw = flipped;
        }
        let err = verify_init_data(&raw, BOT_TOKEN, Duration::from_secs(86_400), now).unwrap_err();
        assert!(matches!(
            err,
            InitDataError::HashMismatch | InitDataError::InvalidHashEncoding
        ));
    }

    #[test]
    fn test_wrong_bot_token_is_rejected() {
        let now = Utc::now();
        let raw = signed_init_data(42, now.timestamp());
        let err =
            verify_init_data(&raw, "999999:not-the-right-token", Duration::from_secs(86_400), now)
                .unwrap_err();
        assert_eq!(err, InitDataError::HashMismatch);
    }

    #[test]
    fn test_expired_auth_date_is_rejected() {
        let now = Utc::now();
        let old = now - chrono::Duration::hours(48);
        let raw = signed_init_data(42, old.timestamp());
        let err = verify_init_data(&raw, BOT_TOKEN, Duration::from_secs(86_400), now).unwrap_err();
        assert_eq!(err, InitDataError::Expired);
    }

    #[test]
    fn test_auth_date_far_in_the_future_is_rejected() {
        let now = Utc::now();
        let future = now + chrono::Duration::hours(1);
        let raw = signed_init_data(42, future.timestamp());
        let err = verify_init_data(&raw, BOT_TOKEN, Duration::from_secs(86_400), now).unwrap_err();
        assert_eq!(err, InitDataError::Expired);
    }

    // "A replayed payload": initData carries no single-use nonce by Telegram's own design (see
    // this module's header comment) -- the same still-fresh string is reused across every
    // request in a session, so verifying it twice must succeed both times, not fail the second
    // time as if it were a consumed token. This documents that behaviour is deliberate rather
    // than an oversight; auth_date recency (covered above) is the actual replay defense.
    #[test]
    fn test_a_still_fresh_payload_can_be_verified_more_than_once() {
        let now = Utc::now();
        let raw = signed_init_data(42, now.timestamp());
        let first = verify_init_data(&raw, BOT_TOKEN, Duration::from_secs(86_400), now).unwrap();
        let second = verify_init_data(&raw, BOT_TOKEN, Duration::from_secs(86_400), now).unwrap();
        assert_eq!(first.id, second.id);
    }

    #[test]
    fn test_empty_header_is_rejected() {
        let err =
            verify_init_data("", BOT_TOKEN, Duration::from_secs(86_400), Utc::now()).unwrap_err();
        assert_eq!(err, InitDataError::Missing);
    }

    #[test]
    fn test_missing_user_field_is_rejected() {
        let now = Utc::now();
        let auth_date = now.timestamp();
        let mut pairs: BTreeMap<String, String> = BTreeMap::new();
        pairs.insert("auth_date".to_string(), auth_date.to_string());
        let data_check_string = pairs
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut secret_mac = Hmac::<Sha256>::new_from_slice(b"WebAppData").unwrap();
        secret_mac.update(BOT_TOKEN.as_bytes());
        let secret_key = secret_mac.finalize().into_bytes();
        let mut mac = Hmac::<Sha256>::new_from_slice(&secret_key).unwrap();
        mac.update(data_check_string.as_bytes());
        let hash = hex_encode(&mac.finalize().into_bytes());
        let raw = format!("auth_date={auth_date}&hash={hash}");

        let err = verify_init_data(&raw, BOT_TOKEN, Duration::from_secs(86_400), now).unwrap_err();
        assert_eq!(err, InitDataError::MissingUser);
    }
}
