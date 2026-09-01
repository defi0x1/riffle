use chrono::{DateTime, Utc};
use eyre::WrapErr;
use sqlx::PgPool;

#[derive(Clone, Debug)]
pub struct NewWallet {
    pub pubkey: String,
    pub telegram_user_id: i64,
    pub label: Option<String>,
    pub registered_at: DateTime<Utc>,
}

// A pubkey belongs to exactly one Telegram user forever (see 0028): this never updates
// `telegram_user_id` on an existing row, only branches on whether the caller already owns it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegisterWalletOutcome {
    Registered,
    // Idempotent re-registration by its own owner -- e.g. relabeling, or un-revoking a wallet
    // the same user removed from their device earlier. Label and revoked_at are refreshed.
    AlreadyOwnedByCaller,
    // The pubkey is already registered to a different Telegram user. The backend cannot verify
    // who actually controls the keypair, so it refuses to reassign ownership rather than
    // guessing; the caller must have the original owner revoke it first.
    OwnedByAnotherUser { owner_telegram_user_id: i64 },
}

pub async fn register_wallet(
    pool: &PgPool,
    row: &NewWallet,
) -> eyre::Result<RegisterWalletOutcome> {
    let mut tx = pool
        .begin()
        .await
        .wrap_err_with(|| "Starting wallet registration transaction")?;

    let existing = sqlx::query!(
        "SELECT telegram_user_id FROM wallets WHERE pubkey = $1 FOR UPDATE",
        row.pubkey,
    )
    .fetch_optional(&mut *tx)
    .await
    .wrap_err_with(|| format!("Looking up wallet {}", row.pubkey))?;

    let outcome = match existing {
        None => {
            sqlx::query!(
                r#"
                INSERT INTO wallets (pubkey, telegram_user_id, label, registered_at)
                VALUES ($1, $2, $3, $4)
                "#,
                row.pubkey,
                row.telegram_user_id,
                row.label,
                row.registered_at,
            )
            .execute(&mut *tx)
            .await
            .wrap_err_with(|| format!("Registering wallet {}", row.pubkey))?;

            RegisterWalletOutcome::Registered
        }
        Some(existing) if existing.telegram_user_id == row.telegram_user_id => {
            sqlx::query!(
                r#"
                UPDATE wallets SET label = $2, revoked_at = NULL WHERE pubkey = $1
                "#,
                row.pubkey,
                row.label,
            )
            .execute(&mut *tx)
            .await
            .wrap_err_with(|| format!("Re-registering wallet {}", row.pubkey))?;

            RegisterWalletOutcome::AlreadyOwnedByCaller
        }
        Some(existing) => RegisterWalletOutcome::OwnedByAnotherUser {
            owner_telegram_user_id: existing.telegram_user_id,
        },
    };

    tx.commit()
        .await
        .wrap_err_with(|| "Committing wallet registration transaction")?;

    Ok(outcome)
}

// Idempotent: revoking an already-revoked wallet again leaves revoked_at at its original value,
// same as close_paper_position's COALESCE pattern.
pub async fn revoke_wallet(
    pool: &PgPool,
    pubkey: &str,
    revoked_at: DateTime<Utc>,
) -> eyre::Result<()> {
    sqlx::query!(
        r#"
        UPDATE wallets SET revoked_at = COALESCE(revoked_at, $2) WHERE pubkey = $1
        "#,
        pubkey,
        revoked_at,
    )
    .execute(pool)
    .await
    .wrap_err_with(|| format!("Revoking wallet {pubkey}"))?;

    Ok(())
}

#[cfg(all(test, feature = "db-tests"))]
mod tests {
    use super::*;
    use crate::test_support::test_pool;

    #[tokio::test]
    async fn test_register_wallet_is_idempotent_for_its_own_owner() {
        let pool = test_pool().await;
        let pubkey = "wallet_register_idempotent_11111111111111111";
        sqlx::query!("DELETE FROM wallets WHERE pubkey = $1", pubkey)
            .execute(&pool)
            .await
            .unwrap();

        let row = NewWallet {
            pubkey: pubkey.to_string(),
            telegram_user_id: 42,
            label: Some("main".to_string()),
            registered_at: Utc::now(),
        };

        let first = register_wallet(&pool, &row).await.unwrap();
        assert_eq!(first, RegisterWalletOutcome::Registered);

        let second = register_wallet(&pool, &row).await.unwrap();
        assert_eq!(second, RegisterWalletOutcome::AlreadyOwnedByCaller);

        let count = sqlx::query_scalar!("SELECT count(*) FROM wallets WHERE pubkey = $1", pubkey)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, Some(1));
    }

    #[tokio::test]
    async fn test_register_wallet_refuses_a_second_owner() {
        let pool = test_pool().await;
        let pubkey = "wallet_register_conflict_1111111111111111111";
        sqlx::query!("DELETE FROM wallets WHERE pubkey = $1", pubkey)
            .execute(&pool)
            .await
            .unwrap();

        register_wallet(
            &pool,
            &NewWallet {
                pubkey: pubkey.to_string(),
                telegram_user_id: 100,
                label: None,
                registered_at: Utc::now(),
            },
        )
        .await
        .unwrap();

        let outcome = register_wallet(
            &pool,
            &NewWallet {
                pubkey: pubkey.to_string(),
                telegram_user_id: 200,
                label: None,
                registered_at: Utc::now(),
            },
        )
        .await
        .unwrap();

        assert_eq!(
            outcome,
            RegisterWalletOutcome::OwnedByAnotherUser {
                owner_telegram_user_id: 100
            }
        );

        let owner = sqlx::query_scalar!(
            "SELECT telegram_user_id FROM wallets WHERE pubkey = $1",
            pubkey
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(owner, 100);
    }

    #[tokio::test]
    async fn test_revoke_wallet_is_idempotent() {
        let pool = test_pool().await;
        let pubkey = "wallet_revoke_idempotent_111111111111111111";
        sqlx::query!("DELETE FROM wallets WHERE pubkey = $1", pubkey)
            .execute(&pool)
            .await
            .unwrap();

        register_wallet(
            &pool,
            &NewWallet {
                pubkey: pubkey.to_string(),
                telegram_user_id: 7,
                label: None,
                registered_at: Utc::now(),
            },
        )
        .await
        .unwrap();

        let first_revoke = Utc::now();
        revoke_wallet(&pool, pubkey, first_revoke).await.unwrap();
        revoke_wallet(&pool, pubkey, first_revoke + chrono::Duration::hours(1))
            .await
            .unwrap();

        let revoked_at =
            sqlx::query_scalar!("SELECT revoked_at FROM wallets WHERE pubkey = $1", pubkey)
                .fetch_one(&pool)
                .await
                .unwrap();

        assert_eq!(
            revoked_at.unwrap().timestamp_millis(),
            first_revoke.timestamp_millis()
        );
    }
}
