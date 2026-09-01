use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use thiserror::Error;

/// Matches `ApiErrorBody` in the miniapp's own `src/api/types.ts` exactly -- `error` is a
/// human-readable message the Mini App can show as-is, `code` is a stable string it can branch
/// on without parsing the message text.
#[derive(Debug, Serialize)]
pub struct ApiErrorBody {
    pub error: String,
    pub code: String,
}

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("{0}")]
    Unauthorized(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{message}")]
    Conflict { message: String, code: &'static str },
    #[error("{0}")]
    BadRequest(String),
    /// A refusal to build something the request asked for, per the task's own list: an unknown
    /// pool, a pool failing the risk gate, an amount beyond the configured cap, a wallet not
    /// registered to the caller. `code` is the stable identifier the Mini App can render
    /// specific copy for.
    #[error("{message}")]
    Refused { message: String, code: &'static str },
    #[error("internal error")]
    Internal(#[from] eyre::Error),
}

impl ApiError {
    pub fn refused(code: &'static str, message: impl Into<String>) -> Self {
        ApiError::Refused {
            message: message.into(),
            code,
        }
    }

    pub fn conflict(code: &'static str, message: impl Into<String>) -> Self {
        ApiError::Conflict {
            message: message.into(),
            code,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message) = match &self {
            ApiError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, "unauthorized", m.clone()),
            ApiError::NotFound(m) => (StatusCode::NOT_FOUND, "not_found", m.clone()),
            ApiError::Conflict { message, code } => (StatusCode::CONFLICT, *code, message.clone()),
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, "bad_request", m.clone()),
            ApiError::Refused { message, code } => {
                (StatusCode::UNPROCESSABLE_ENTITY, *code, message.clone())
            }
            ApiError::Internal(e) => {
                // The only place a full error chain is logged -- the response body never
                // carries more than a generic message, so an internal failure detail (which
                // could echo back a fragment of a request) never reaches the client.
                tracing::error!(error = ?e, "Internal error handling request");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "internal error".to_string(),
                )
            }
        };

        (
            status,
            Json(ApiErrorBody {
                error: message,
                code: code.to_string(),
            }),
        )
            .into_response()
    }
}
