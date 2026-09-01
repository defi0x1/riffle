mod positions;
mod submit;
mod tx;
mod wallet;

use axum::Router;
use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{get, post};
use chrono::Utc;
use tracing::Instrument;

use crate::error::ApiError;
use crate::http_metrics::{HTTP_REQUEST_DURATION_SECS, HTTP_REQUESTS_TOTAL};
use crate::state::AppState;
use crate::telegram_auth::{self, TelegramUser};

pub const INIT_DATA_HEADER: &str = "x-telegram-init-data";

/// The gate on every authenticated endpoint. Never trusts a Telegram user id from anywhere but
/// this -- no handler in this crate reads a user id out of a request body.
async fn authenticate(headers: &HeaderMap, state: &AppState) -> Result<TelegramUser, ApiError> {
    let raw = headers
        .get(INIT_DATA_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    telegram_auth::verify_init_data(raw, &state.config.bot_token, state.config.init_data_max_age, Utc::now())
        .map_err(|e| ApiError::Unauthorized(e.to_string()))
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/wallet/register", post(wallet::register))
        .route("/api/v1/wallet/balances", get(wallet::balances))
        .route("/api/v1/positions", get(positions::positions))
        .route("/api/v1/positions/{position_address}/profit", get(positions::profit))
        .route("/api/v1/tx/open-position", post(tx::open_position))
        .route("/api/v1/tx/add-liquidity", post(tx::add_liquidity))
        .route("/api/v1/tx/remove-liquidity", post(tx::remove_liquidity))
        .route("/api/v1/tx/claim-fees", post(tx::claim_fees))
        .route("/api/v1/tx/close-position", post(tx::close_position))
        .route("/api/v1/tx/submit", post(submit::submit))
        .route("/api/v1/tx/status", get(submit::status))
        .layer(middleware::from_fn(request_tracing))
        .with_state(state)
}

/// Structured tracing with a request id on every request, per this task's own requirement. Logs
/// only method, path, status and duration -- never a request or response body, which could
/// carry a signed transaction blob or the raw `initData` header. The Telegram user id, once a
/// handler has authenticated the request, is logged separately by that handler, since it is the
/// one piece of user identification this service is explicitly allowed to log.
async fn request_tracing(req: Request, next: Next) -> Response {
    let request_id = uuid::Uuid::new_v4().to_string();
    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    let span = tracing::info_span!("http_request", request_id = %request_id, %method, %path);

    async move {
        let start = std::time::Instant::now();
        let mut response = next.run(req).await;
        let elapsed = start.elapsed();
        let status = response.status();

        if let Ok(value) = HeaderValue::from_str(&request_id) {
            response.headers_mut().insert("x-request-id", value);
        }

        HTTP_REQUESTS_TOTAL
            .with_label_values(&[method.as_str(), &path, status.as_str()])
            .inc();
        HTTP_REQUEST_DURATION_SECS
            .with_label_values(&[method.as_str(), &path])
            .observe(elapsed.as_secs_f64());

        tracing::info!(status = %status, elapsed_ms = elapsed.as_millis(), "Request completed");
        response
    }
    .instrument(span)
    .await
}
