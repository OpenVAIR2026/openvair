//! Password reset by an emailed code for users who cannot sign in.
use crate::{
    AppState, audit, auth, email, email_change,
    error::{AppError, Result},
};
use axum::{Json, extract::State, http::HeaderMap};
use chrono::{Duration, Utc};
use rand::{Rng, rngs::OsRng};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestInput {
    email: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmInput {
    request_id: Uuid,
    email: String,
    code: String,
    new_password: String,
}

const INVALID_CODE: &str = "验证码无效或已过期，请重新获取";

/// Responds the same way whether or not the email is registered.
pub async fn request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<RequestInput>,
) -> Result<Json<Value>> {
    let email = auth::normalize_email(&input.email)?;
    let smtp = email::load_smtp_config(&state).await?;
    if let Some(ip) = auth::client_ip(&headers) {
        email_change::limit(
            &state,
            &format!("reset-ip:{}", auth::hash_token(&ip)),
            20,
            3600,
        )
        .await?;
    }
    let recipient = auth::hash_token(&email);
    email_change::limit(&state, &format!("reset-cooldown:{recipient}"), 1, 60).await?;
    email_change::limit(&state, &format!("reset-recipient:{recipient}"), 5, 3600).await?;
    let id = Uuid::new_v4();
    let code = format!("{:06}", OsRng.gen_range(0..1_000_000_u32));
    let code_to_hash = code.clone();
    // Hash before the lookup so unknown emails take as long as registered ones.
    let code_hash = tokio::task::spawn_blocking(move || auth::hash_password(&code_to_hash))
        .await
        .map_err(|_| AppError::Internal("code hashing failed".into()))??;
    let user_id: Option<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE email=$1")
        .bind(&email)
        .fetch_optional(&state.db)
        .await?;
    if let Some(user_id) = user_id {
        sqlx::query(
            "INSERT INTO password_resets(user_id,id,code_hash,expires_at) VALUES ($1,$2,$3,$4)
             ON CONFLICT (user_id) DO UPDATE SET id=EXCLUDED.id,code_hash=EXCLUDED.code_hash,
             attempts=0,expires_at=EXCLUDED.expires_at",
        )
        .bind(user_id)
        .bind(id)
        .bind(code_hash)
        .bind(Utc::now() + Duration::minutes(10))
        .execute(&state.db)
        .await?;
        // Delivery runs in the background so its duration does not reveal the account.
        let db = state.db.clone();
        tokio::spawn(async move {
            if email::send_password_reset_code(smtp, email, code)
                .await
                .is_err()
            {
                let _ = sqlx::query("DELETE FROM password_resets WHERE user_id=$1 AND id=$2")
                    .bind(user_id)
                    .bind(id)
                    .execute(&db)
                    .await;
            }
        });
    }
    Ok(Json(
        json!({"request_id":id,"expires_in":600,"retry_after":60}),
    ))
}

pub async fn confirm(
    State(state): State<AppState>,
    Json(input): Json<ConfirmInput>,
) -> Result<Json<Value>> {
    if auth::validate_password(&input.new_password).is_err() {
        return Err(AppError::BadRequest("新密码长度需为 10–256 个字符".into()));
    }
    let email = auth::normalize_email(&input.email)?;
    let mut tx = state.db.begin().await?;
    let user_id: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE email=$1 FOR UPDATE")
        .bind(&email)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::BadRequest(INVALID_CODE.into()))?;
    let (id, code_hash, attempts): (Uuid, String, i32) = sqlx::query_as(
        "SELECT id,code_hash,attempts FROM password_resets WHERE user_id=$1 AND expires_at>now() FOR UPDATE",
    )
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::BadRequest(INVALID_CODE.into()))?;
    if id != input.request_id {
        return Err(AppError::BadRequest(INVALID_CODE.into()));
    }
    if attempts >= 5 {
        return Err(AppError::BadRequest(
            "验证码错误次数过多，请重新获取".into(),
        ));
    }
    let code = input.code;
    let matches = code.len() == 6
        && code.bytes().all(|byte| byte.is_ascii_digit())
        && tokio::task::spawn_blocking(move || auth::verify_password(&code, &code_hash))
            .await
            .map_err(|_| AppError::Internal("code check failed".into()))?;
    if !matches {
        sqlx::query("UPDATE password_resets SET attempts=attempts+1 WHERE user_id=$1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Err(AppError::BadRequest("验证码不正确，请重新输入".into()));
    }
    let new_password = input.new_password;
    let password_hash = tokio::task::spawn_blocking(move || auth::hash_password(&new_password))
        .await
        .map_err(|_| AppError::Internal("password hashing failed".into()))??;
    sqlx::query("UPDATE users SET password_hash=$1 WHERE id=$2")
        .bind(password_hash)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    // A reset signs out every device, including any held by an attacker.
    for statement in [
        "DELETE FROM password_resets WHERE user_id=$1",
        "DELETE FROM sessions WHERE user_id=$1",
        "DELETE FROM email_changes WHERE user_id=$1",
    ] {
        sqlx::query(statement)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
    }
    auth::revoke_client_tokens(&mut *tx, user_id).await?;
    tx.commit().await?;
    auth::clear_login_failures(&state, &email).await;
    email::notify_in_background(&state, email, email::SecurityNotice::PasswordChanged);
    let account_id: Option<Uuid> =
        sqlx::query_scalar("SELECT account_id FROM memberships WHERE user_id=$1")
            .bind(user_id)
            .fetch_optional(&state.db)
            .await?;
    if let Some(account_id) = account_id {
        audit(
            &state.db,
            account_id,
            user_id,
            "user.password_reset",
            "user",
            user_id,
            json!({}),
        )
        .await;
    }
    Ok(Json(json!({"ok":true})))
}
