use crate::{
    AppState, audit, auth, email,
    error::{AppError, Result},
};
use axum::{Json, extract::State, http::HeaderMap};
use chrono::{DateTime, Duration, Utc};
use rand::{Rng, rngs::OsRng};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestInput {
    email: String,
    current_password: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmInput {
    request_id: Uuid,
    email: String,
    code: String,
}

#[derive(FromRow)]
struct Challenge {
    id: Uuid,
    session_hash: String,
    current_email: String,
    new_email: String,
    code_hash: String,
    attempts: i32,
    ready: bool,
    expires_at: DateTime<Utc>,
}

fn session_hash(headers: &HeaderMap) -> Result<String> {
    auth::read_cookie(headers, "gateway_session")
        .map(|token| auth::hash_token(&token))
        .ok_or(AppError::Unauthorized)
}

fn normalize_target(value: &str) -> Result<String> {
    let email = auth::normalize_email(value)?;
    // Parse only an address, never a display-name or a list of recipients.
    email
        .parse::<lettre::Address>()
        .map_err(|_| AppError::BadRequest("请输入有效的邮箱地址".into()))?;
    Ok(email)
}

// Atomic counters prevent concurrent requests and changing recipients from bypassing limits.
pub(crate) async fn limit(state: &AppState, key: &str, maximum: i64, seconds: i64) -> Result<()> {
    let mut redis = state
        .redis
        .get_multiplexed_async_connection()
        .await
        .map_err(|_| AppError::ServiceUnavailable("验证服务暂不可用，请稍后重试".into()))?;
    let count: i64 = redis::Script::new(
        "local n = redis.call('INCR', KEYS[1]); if n == 1 then redis.call('EXPIRE', KEYS[1], ARGV[1]) end; return n"
    ).key(format!("meta-router:email-change:{key}")).arg(seconds)
        .invoke_async(&mut redis).await
        .map_err(|_| AppError::ServiceUnavailable("验证服务暂不可用，请稍后重试".into()))?;
    if count > maximum {
        return Err(AppError::RateLimited);
    }
    Ok(())
}

pub async fn request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<RequestInput>,
) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    let smtp = email::load_smtp_config(&state).await?;
    let target = normalize_target(&input.email)?;
    if target == user.email {
        return Err(AppError::BadRequest("请输入与当前邮箱不同的新邮箱".into()));
    }
    limit(&state, &format!("password:{}", user.user_id), 10, 600).await?;
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id=$1")
        .bind(user.user_id)
        .fetch_one(&state.db)
        .await?;
    let valid_password =
        tokio::task::spawn_blocking(move || auth::verify_password(&input.current_password, &hash))
            .await
            .map_err(|_| AppError::Internal("password check failed".into()))?;
    if !valid_password {
        return Err(AppError::BadRequest("当前密码不正确".into()));
    }
    if sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM users WHERE email=$1)")
        .bind(&target)
        .fetch_one(&state.db)
        .await?
    {
        return Err(AppError::Conflict("该邮箱已被使用".into()));
    }
    limit(&state, &format!("cooldown:{}", user.user_id), 1, 60).await?;
    limit(&state, &format!("hour:{}", user.user_id), 10, 3600).await?;
    limit(
        &state,
        &format!("recipient:{}", auth::hash_token(&target)),
        5,
        3600,
    )
    .await?;
    let code = format!("{:06}", OsRng.gen_range(0..1_000_000_u32));
    let code_to_hash = code.clone();
    let code_hash = tokio::task::spawn_blocking(move || auth::hash_password(&code_to_hash))
        .await
        .map_err(|_| AppError::Internal("code hashing failed".into()))??;
    let id = Uuid::new_v4();
    let mut tx = state.db.begin().await?;
    let current: String = sqlx::query_scalar("SELECT email FROM users WHERE id=$1 FOR UPDATE")
        .bind(user.user_id)
        .fetch_one(&mut *tx)
        .await?;
    if current != user.email {
        return Err(AppError::Conflict("邮箱已变更，请刷新后重试".into()));
    }
    sqlx::query(
        "INSERT INTO email_changes(user_id,id,session_hash,current_email,new_email,code_hash,expires_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7)
         ON CONFLICT (user_id) DO UPDATE SET id=EXCLUDED.id,session_hash=EXCLUDED.session_hash,
         current_email=EXCLUDED.current_email,new_email=EXCLUDED.new_email,code_hash=EXCLUDED.code_hash,
         attempts=0,ready=false,expires_at=EXCLUDED.expires_at"
    ).bind(user.user_id).bind(id).bind(session_hash(&headers)?).bind(&current).bind(&target)
        .bind(code_hash).bind(Utc::now() + Duration::minutes(10)).execute(&mut *tx).await?;
    tx.commit().await?;
    // A failed send never leaves an active challenge, and its error never contains SMTP credentials.
    if let Err(error) = email::send_verification_code(smtp, target, code).await {
        sqlx::query("DELETE FROM email_changes WHERE user_id=$1 AND id=$2")
            .bind(user.user_id)
            .bind(id)
            .execute(&state.db)
            .await?;
        return Err(error);
    }
    let updated = sqlx::query("UPDATE email_changes SET ready=true WHERE user_id=$1 AND id=$2")
        .bind(user.user_id)
        .bind(id)
        .execute(&state.db)
        .await?;
    if updated.rows_affected() != 1 {
        return Err(AppError::Conflict("请求已失效，请重新发送验证码".into()));
    }
    Ok(Json(
        json!({"request_id":id,"expires_in":600,"retry_after":60}),
    ))
}

pub async fn confirm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<ConfirmInput>,
) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    let target = normalize_target(&input.email)?;
    let token_hash = session_hash(&headers)?;
    let mut tx = state.db.begin().await?;
    // Always lock the user before the challenge, matching the send path.
    let current: String = sqlx::query_scalar("SELECT email FROM users WHERE id=$1 FOR UPDATE")
        .bind(user.user_id)
        .fetch_one(&mut *tx)
        .await?;
    let challenge = sqlx::query_as::<_, Challenge>(
        "SELECT id,session_hash,current_email,new_email,code_hash,attempts,ready,expires_at
         FROM email_changes WHERE user_id=$1 FOR UPDATE",
    )
    .bind(user.user_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::BadRequest("验证码无效或已过期，请重新发送".into()))?;
    if !challenge.ready
        || challenge.id != input.request_id
        || challenge.session_hash != token_hash
        || challenge.current_email != current
        || challenge.new_email != target
        || challenge.expires_at <= Utc::now()
    {
        return Err(AppError::BadRequest(
            "验证码无效或已过期，请重新发送".into(),
        ));
    }
    if challenge.attempts >= 5 {
        return Err(AppError::BadRequest(
            "验证码错误次数过多，请重新发送".into(),
        ));
    }
    let valid_format =
        input.code.len() == 6 && input.code.bytes().all(|byte| byte.is_ascii_digit());
    let matches = valid_format
        && tokio::task::spawn_blocking(move || {
            auth::verify_password(&input.code, &challenge.code_hash)
        })
        .await
        .map_err(|_| AppError::Internal("code check failed".into()))?;
    if !matches {
        sqlx::query("UPDATE email_changes SET attempts=attempts+1 WHERE user_id=$1")
            .bind(user.user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Err(AppError::BadRequest("验证码不正确，请重新输入".into()));
    }
    sqlx::query("UPDATE users SET email=$1 WHERE id=$2")
        .bind(&target)
        .bind(user.user_id)
        .execute(&mut *tx)
        .await
        .map_err(|error| {
            if error
                .as_database_error()
                .is_some_and(|error| error.is_unique_violation())
            {
                AppError::Conflict("该邮箱已被使用".into())
            } else {
                error.into()
            }
        })?;
    sqlx::query("DELETE FROM email_changes WHERE user_id=$1")
        .bind(user.user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id=$1 AND token_hash<>$2")
        .bind(user.user_id)
        .bind(&token_hash)
        .execute(&mut *tx)
        .await?;
    auth::revoke_client_tokens(&mut *tx, user.user_id).await?;
    tx.commit().await?;
    // Tell the previous address, so an unauthorized change does not go unnoticed.
    email::notify_in_background(
        &state,
        challenge.current_email.clone(),
        email::SecurityNotice::EmailChanged {
            new_email: target.clone(),
        },
    );
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "user.email_updated",
        "user",
        user.user_id,
        json!({}),
    )
    .await;
    Ok(Json(json!(auth::current_user(&state, &headers).await?)))
}
