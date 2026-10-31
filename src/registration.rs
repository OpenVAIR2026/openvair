//! Email verification for new sign-ups: a code proves the address before the
//! account exists, so nobody can claim someone else's email.
use crate::{
    AppState, auth, email, email_change,
    error::{AppError, Result},
    system_admin,
};
use axum::{Json, extract::State, http::HeaderMap};
use chrono::{Duration, Utc};
use rand::{Rng, rngs::OsRng};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgConnection;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeInput {
    email: String,
}

const INVALID_CODE: &str = "验证码无效或已过期，请重新获取";

/// Responds the same way whether or not the email is registered: new addresses
/// get a code, registered ones a reminder that they already have an account.
pub async fn request_code(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CodeInput>,
) -> Result<Json<Value>> {
    let email = auth::normalize_email(&input.email)?;
    system_admin::ensure_registration_allowed(&state, &email).await?;
    let settings = system_admin::load_registration(&state.db).await;
    if !system_admin::email_verification_required(&state, &settings).await {
        return Err(AppError::BadRequest("当前注册无需邮箱验证".into()));
    }
    let smtp = email::load_smtp_config(&state).await?;
    if let Some(ip) = auth::client_ip(&headers) {
        email_change::limit(
            &state,
            &format!("register-ip:{}", auth::hash_token(&ip)),
            20,
            3600,
        )
        .await?;
    }
    let recipient = auth::hash_token(&email);
    email_change::limit(&state, &format!("register-cooldown:{recipient}"), 1, 60).await?;
    email_change::limit(&state, &format!("register-recipient:{recipient}"), 5, 3600).await?;
    let id = Uuid::new_v4();
    let code = format!("{:06}", OsRng.gen_range(0..1_000_000_u32));
    let code_to_hash = code.clone();
    // Hash before the lookup so registered emails take as long as new ones.
    let code_hash = tokio::task::spawn_blocking(move || auth::hash_password(&code_to_hash))
        .await
        .map_err(|_| AppError::Internal("code hashing failed".into()))??;
    let registered: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE email=$1)")
        .bind(&email)
        .fetch_one(&state.db)
        .await?;
    if registered {
        email::send_registration_existing(smtp, email).await?;
    } else {
        sqlx::query(
            "INSERT INTO registration_verifications(email,id,code_hash,expires_at) VALUES ($1,$2,$3,$4)
             ON CONFLICT (email) DO UPDATE SET id=EXCLUDED.id,code_hash=EXCLUDED.code_hash,
             attempts=0,expires_at=EXCLUDED.expires_at",
        )
        .bind(&email)
        .bind(id)
        .bind(code_hash)
        .bind(Utc::now() + Duration::minutes(10))
        .execute(&state.db)
        .await?;
        if let Err(error) = email::send_registration_code(smtp, email.clone(), code).await {
            let _ = sqlx::query("DELETE FROM registration_verifications WHERE email=$1 AND id=$2")
                .bind(&email)
                .bind(id)
                .execute(&state.db)
                .await;
            return Err(error);
        }
    }
    Ok(Json(
        json!({"request_id":id,"expires_in":600,"retry_after":60}),
    ))
}

/// Outcome of checking a sign-up code inside the registration transaction.
pub enum Check {
    Verified,
    /// The caller commits so a wrong guess still counts, then returns the error.
    Rejected(AppError),
}

pub async fn consume_code(
    conn: &mut PgConnection,
    email: &str,
    request_id: Option<Uuid>,
    code: Option<String>,
) -> Result<Check> {
    let (Some(request_id), Some(code)) = (request_id, code) else {
        return Ok(Check::Rejected(AppError::BadRequest(
            "请先获取并填写邮箱验证码".into(),
        )));
    };
    let row: Option<(Uuid, String, i32)> = sqlx::query_as(
        "SELECT id,code_hash,attempts FROM registration_verifications
         WHERE email=$1 AND expires_at>now() FOR UPDATE",
    )
    .bind(email)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((id, code_hash, attempts)) = row else {
        return Ok(Check::Rejected(AppError::BadRequest(INVALID_CODE.into())));
    };
    // A code requested for another address or an older request never matches.
    if id != request_id {
        return Ok(Check::Rejected(AppError::BadRequest(INVALID_CODE.into())));
    }
    if attempts >= 5 {
        return Ok(Check::Rejected(AppError::BadRequest(
            "验证码错误次数过多，请重新获取".into(),
        )));
    }
    let matches = code.len() == 6
        && code.bytes().all(|byte| byte.is_ascii_digit())
        && tokio::task::spawn_blocking(move || auth::verify_password(&code, &code_hash))
            .await
            .map_err(|_| AppError::Internal("code check failed".into()))?;
    if !matches {
        sqlx::query("UPDATE registration_verifications SET attempts=attempts+1 WHERE email=$1")
            .bind(email)
            .execute(&mut *conn)
            .await?;
        return Ok(Check::Rejected(AppError::BadRequest(
            "验证码不正确，请重新输入".into(),
        )));
    }
    sqlx::query("DELETE FROM registration_verifications WHERE email=$1")
        .bind(email)
        .execute(&mut *conn)
        .await?;
    Ok(Check::Verified)
}
