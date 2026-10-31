//! The signed-in user's sessions, so other devices can be recognized and signed out.
use crate::{
    AppState, audit, auth,
    error::{AppError, Result},
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

#[derive(Debug, Serialize, FromRow)]
pub struct SessionView {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub expires_at: DateTime<Utc>,
    pub user_agent: Option<String>,
    pub current: bool,
}

fn current_hash(headers: &HeaderMap) -> Result<String> {
    auth::read_cookie(headers, "gateway_session")
        .map(|token| auth::hash_token(&token))
        .ok_or(AppError::Unauthorized)
}

/// Records activity at most every five minutes to keep reads cheap.
pub async fn touch(db: &PgPool, headers: &HeaderMap) {
    if let Ok(token_hash) = current_hash(headers) {
        let _ = sqlx::query(
            "UPDATE sessions SET last_seen_at=now() WHERE token_hash=$1
             AND (last_seen_at IS NULL OR last_seen_at < now()-interval '5 minutes')",
        )
        .bind(token_hash)
        .execute(db)
        .await;
    }
}

pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<SessionView>>> {
    let user = auth::current_user(&state, &headers).await?;
    let token_hash = current_hash(&headers)?;
    Ok(Json(
        sqlx::query_as(
            "SELECT id,created_at,last_seen_at,expires_at,user_agent,token_hash=$2 AS current
             FROM sessions WHERE user_id=$1 AND expires_at>now()
             ORDER BY token_hash=$2 DESC, COALESCE(last_seen_at,created_at) DESC, id",
        )
        .bind(user.user_id)
        .bind(token_hash)
        .fetch_all(&state.db)
        .await?,
    ))
}

pub async fn revoke(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let user = auth::current_user(&state, &headers).await?;
    let token_hash = current_hash(&headers)?;
    let current: Option<bool> = sqlx::query_scalar(
        "DELETE FROM sessions WHERE id=$1 AND user_id=$2 AND token_hash<>$3 RETURNING false",
    )
    .bind(id)
    .bind(user.user_id)
    .bind(&token_hash)
    .fetch_optional(&state.db)
    .await?;
    if current.is_none() {
        let is_current: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=$1 AND user_id=$2 AND token_hash=$3)",
        )
        .bind(id)
        .bind(user.user_id)
        .bind(&token_hash)
        .fetch_one(&state.db)
        .await?;
        return Err(if is_current {
            AppError::BadRequest("当前设备请使用“退出登录”".into())
        } else {
            AppError::NotFound
        });
    }
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "user.session_revoked",
        "session",
        id,
        json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn revoke_others(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    let token_hash = current_hash(&headers)?;
    let revoked = sqlx::query("DELETE FROM sessions WHERE user_id=$1 AND token_hash<>$2")
        .bind(user.user_id)
        .bind(token_hash)
        .execute(&state.db)
        .await?
        .rows_affected();
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "user.sessions_revoked",
        "user",
        user.user_id,
        json!({"count": revoked}),
    )
    .await;
    Ok(Json(json!({"revoked": revoked})))
}
