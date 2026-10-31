//! Password changes for the signed-in user.
use crate::{
    AppState, audit, auth, email, email_change,
    error::{AppError, Result},
};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeInput {
    current_password: String,
    new_password: String,
}

pub async fn change(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<ChangeInput>,
) -> Result<StatusCode> {
    let user = auth::current_user(&state, &headers).await?;
    let session_hash = auth::read_cookie(&headers, "gateway_session")
        .map(|token| auth::hash_token(&token))
        .ok_or(AppError::Unauthorized)?;
    if auth::validate_password(&input.new_password).is_err() {
        return Err(AppError::BadRequest("新密码长度需为 10–256 个字符".into()));
    }
    if input.new_password == input.current_password {
        return Err(AppError::BadRequest("新密码不能与当前密码相同".into()));
    }
    // Shares the password-check budget with email changes.
    email_change::limit(&state, &format!("password:{}", user.user_id), 10, 600).await?;
    let current_hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id=$1")
        .bind(user.user_id)
        .fetch_one(&state.db)
        .await?;
    let ChangeInput {
        current_password,
        new_password,
    } = input;
    let verified_hash = current_hash.clone();
    let valid_password = tokio::task::spawn_blocking(move || {
        auth::verify_password(&current_password, &verified_hash)
    })
    .await
    .map_err(|_| AppError::Internal("password check failed".into()))?;
    if !valid_password {
        return Err(AppError::BadRequest("当前密码不正确".into()));
    }
    let new_hash = tokio::task::spawn_blocking(move || auth::hash_password(&new_password))
        .await
        .map_err(|_| AppError::Internal("password hashing failed".into()))??;
    let mut tx = state.db.begin().await?;
    // Only replace the hash that was verified, so a concurrent change is not overwritten.
    let updated = sqlx::query("UPDATE users SET password_hash=$1 WHERE id=$2 AND password_hash=$3")
        .bind(&new_hash)
        .bind(user.user_id)
        .bind(&current_hash)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if updated == 0 {
        return Err(AppError::Conflict("密码已被修改，请刷新后重试".into()));
    }
    // Other devices and pending email changes were authorized with the old password.
    sqlx::query("DELETE FROM sessions WHERE user_id=$1 AND token_hash<>$2")
        .bind(user.user_id)
        .bind(&session_hash)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM email_changes WHERE user_id=$1")
        .bind(user.user_id)
        .execute(&mut *tx)
        .await?;
    auth::revoke_client_tokens(&mut *tx, user.user_id).await?;
    tx.commit().await?;
    email::notify_in_background(
        &state,
        user.email.clone(),
        email::SecurityNotice::PasswordChanged,
    );
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "user.password_updated",
        "user",
        user.user_id,
        json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
#[path = "password_change_db_tests.rs"]
mod db_tests;
