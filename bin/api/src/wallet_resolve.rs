//! Resolves "the wallet" a request acts on -- the miniapp contract never carries a wallet pubkey
//! in any authenticated request or response body (see `miniapp/src/api/client.ts`: `getBalances`
//! and `getPositions` take no argument, and none of the five build-tx requests carry a pubkey
//! either). The custody model is one wallet per device, generated or imported client-side and
//! registered via POST /wallet/register on every app launch, so the most recently registered
//! active wallet for the caller's Telegram identity is the one currently in use on this device.

use storage::queries::WalletRow;

use crate::error::ApiError;
use crate::state::AppState;

pub async fn resolve_wallet(state: &AppState, telegram_user_id: i64) -> Result<WalletRow, ApiError> {
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
