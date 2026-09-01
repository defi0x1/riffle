//! Resolves "the wallet" a request acts on -- the miniapp contract never carries a wallet pubkey
//! in any authenticated request or response body (see `miniapp/src/api/client.ts`: `getBalances`
//! and `getPositions` take no argument, and none of the five build-tx requests carry a pubkey
//! either). The custody model is one wallet per device, generated or imported client-side and
//! registered via POST /wallet/register on every app launch, so the most recently registered
//! active wallet for the caller's Telegram identity is the one currently in use on this device.

use storage::queries::WalletRow;

use crate::error::ApiError;
use crate::state::AppState;

pub async fn resolve_wallet(
    state: &AppState,
    telegram_user_id: i64,
) -> Result<WalletRow, ApiError> {
    let mut wallets = storage::queries::active_wallets_for_user(&state.db, telegram_user_id)
        .await
        .map_err(ApiError::Internal)?;

    // active_wallets_for_user orders by registered_at ascending; the most recent registration
    // is the wallet currently active on this device (see module comment).
    wallets.pop().ok_or_else(|| {
        ApiError::refused(
            "wallet_not_registered",
            "No wallet is registered for this Telegram account yet -- call wallet/register first",
        )
    })
}

#[cfg(all(test, feature = "db-tests"))]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::test_support::{test_pool, test_state};

    // A fresh id per call rather than a fixed literal: this crate has no delete/reset helper of
    // its own (all SQL stays in libraries/storage), so a repeated run against a persistent test
    // database must not collide with rows an earlier run left behind.
    fn unique_telegram_user_id() -> i64 {
        (uuid::Uuid::new_v4().as_u128() % (i64::MAX as u128)) as i64
    }

    #[tokio::test]
    async fn test_unregistered_telegram_user_is_refused() {
        let pool = test_pool().await;
        let state = test_state(pool);

        let err = resolve_wallet(&state, unique_telegram_user_id())
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            ApiError::Refused {
                code: "wallet_not_registered",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn test_most_recently_registered_wallet_is_resolved() {
        let telegram_user_id = unique_telegram_user_id();
        let pool = test_pool().await;
        let first = format!("wallet_resolve_first_{}", uuid::Uuid::new_v4());
        let second = format!("wallet_resolve_second_{}", uuid::Uuid::new_v4());

        storage::write::register_wallet(
            &pool,
            &storage::write::NewWallet {
                pubkey: first.clone(),
                telegram_user_id,
                label: None,
                registered_at: Utc::now(),
            },
        )
        .await
        .unwrap();
        storage::write::register_wallet(
            &pool,
            &storage::write::NewWallet {
                pubkey: second.clone(),
                telegram_user_id,
                label: None,
                registered_at: Utc::now() + chrono::Duration::seconds(1),
            },
        )
        .await
        .unwrap();

        let state = test_state(pool);
        let resolved = resolve_wallet(&state, telegram_user_id).await.unwrap();
        assert_eq!(resolved.pubkey, second);
    }
}
