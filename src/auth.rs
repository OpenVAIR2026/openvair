use crate::{
    error::{AppError, Result},
    state::AppState,
};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use axum::http::{
    HeaderMap,
    header::{SET_COOKIE, USER_AGENT},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgExecutor};
use std::sync::LazyLock;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, FromRow)]
pub struct CurrentUser {
    pub user_id: Uuid,
    pub email: String,
    pub account_id: Uuid,
    pub is_super_admin: bool,
}

#[derive(Debug, Deserialize)]
pub struct RegisterInput {
    pub email: String,
    pub password: String,
    pub account_name: String,
    /// Required when registration asks for email verification.
    #[serde(default)]
    pub request_id: Option<Uuid>,
    #[serde(default)]
    pub code: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginInput {
    pub email: String,
    pub password: String,
}

pub fn normalize_email(value: &str) -> Result<String> {
    let email = value.trim().to_ascii_lowercase();
    if email.len() < 5 || email.len() > 320 || !email.contains('@') {
        return Err(AppError::BadRequest("valid email is required".into()));
    }
    Ok(email)
}

pub fn validate_password(value: &str) -> Result<()> {
    if value.len() < 10 || value.len() > 256 {
        return Err(AppError::BadRequest(
            "password must be 10-256 characters".into(),
        ));
    }
    Ok(())
}

pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|v| v.to_string())
        .map_err(|_| AppError::Internal("password hashing failed".into()))
}

pub fn verify_password(password: &str, encoded: &str) -> bool {
    PasswordHash::new(encoded).is_ok_and(|hash| {
        Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    })
}

pub fn random_token(prefix: &str) -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    format!("{prefix}_{}", URL_SAFE_NO_PAD.encode(bytes))
}

pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub fn session_cookie(token: &str, secure: bool) -> String {
    let secure_flag = if secure { "; Secure" } else { "" };
    format!("gateway_session={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age=604800{secure_flag}")
}

pub fn read_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get("cookie")?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|part| {
            part.trim()
                .strip_prefix(&format!("{name}="))
                .map(ToOwned::to_owned)
        })
}

/// Every user owns exactly one account, created at registration.
pub async fn current_user(state: &AppState, headers: &HeaderMap) -> Result<CurrentUser> {
    let token = read_cookie(headers, "gateway_session").ok_or(AppError::Unauthorized)?;
    let mut user = sqlx::query_as::<_, CurrentUser>(
        "SELECT u.id AS user_id, u.email, m.account_id, u.is_super_admin \
         FROM sessions s JOIN users u ON u.id=s.user_id JOIN memberships m ON m.user_id=u.id \
         JOIN accounts a ON a.id=m.account_id \
         WHERE s.token_hash=$1 AND s.expires_at>now() AND a.disabled_at IS NULL",
    )
    .bind(hash_token(&token))
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::Unauthorized)?;
    if state.super_admin_emails.contains(&user.email) {
        user.is_super_admin = true;
    }
    Ok(user)
}

/// A desktop client signed in through the device grant.
#[derive(Clone, Debug)]
pub struct CurrentClient {
    pub user: CurrentUser,
    pub token_id: Uuid,
    pub client_name: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

/// Client tokens arrive as `Authorization: Bearer vcl_…` and never as a
/// cookie, so they cannot reach the console API.
pub async fn current_client(state: &AppState, headers: &HeaderMap) -> Result<CurrentClient> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| value.starts_with("vcl_"))
        .ok_or(AppError::Unauthorized)?;
    let row: (Uuid, String, Uuid, bool, Uuid, String, chrono::DateTime<chrono::Utc>) =
        sqlx::query_as(
            "UPDATE client_tokens t SET last_seen_at=now(),
               expires_at=CASE WHEN t.last_seen_at IS NULL OR t.last_seen_at < now()-interval '1 hour'
                               THEN now()+interval '30 days' ELSE t.expires_at END
             FROM users u JOIN memberships m ON m.user_id=u.id JOIN accounts a ON a.id=m.account_id
             WHERE u.id=t.user_id AND t.token_hash=$1 AND t.revoked_at IS NULL AND t.expires_at>now()
               AND a.disabled_at IS NULL
             RETURNING u.id, u.email, m.account_id, u.is_super_admin, t.id, t.client_name, t.expires_at",
        )
        .bind(hash_token(token))
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::Unauthorized)?;
    let (user_id, email, account_id, is_super_admin, token_id, client_name, expires_at) = row;
    Ok(CurrentClient {
        user: CurrentUser {
            user_id,
            email,
            account_id,
            is_super_admin,
        },
        token_id,
        client_name,
        expires_at,
    })
}

/// Signing out everywhere (password change or reset) also signs out clients.
pub async fn revoke_client_tokens<'e>(db: impl PgExecutor<'e>, user_id: Uuid) -> Result<()> {
    sqlx::query(
        "UPDATE client_tokens SET revoked_at=now() WHERE user_id=$1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn require_super_admin(state: &AppState, headers: &HeaderMap) -> Result<CurrentUser> {
    let user = current_user(state, headers).await?;
    if !user.is_super_admin {
        return Err(AppError::Forbidden);
    }
    Ok(user)
}

/// Whether the session cookie gets `Secure`. Browsers drop such cookies on
/// plain-HTTP origins, so a console opened on localhost still signs in.
pub fn cookie_secure(configured: bool, request_headers: &HeaderMap) -> bool {
    if !configured {
        return false;
    }
    let host = request_headers
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let host = match host.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or_default(),
        None => host.split(':').next().unwrap_or_default(),
    };
    !(host == "localhost" || host.ends_with(".localhost") || host == "127.0.0.1" || host == "::1")
}

pub fn set_cookie(headers: &mut HeaderMap, token: &str, secure: bool) {
    if let Ok(value) = session_cookie(token, secure).parse() {
        headers.insert(SET_COOKIE, value);
    }
}

pub fn clear_cookie(headers: &mut HeaderMap, secure: bool) {
    let secure_flag = if secure { "; Secure" } else { "" };
    if let Ok(value) =
        format!("gateway_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure_flag}").parse()
    {
        headers.insert(SET_COOKIE, value);
    }
}

/// Creates a seven-day session and returns its raw token for the cookie.
pub async fn insert_session<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
    headers: &HeaderMap,
) -> Result<String> {
    let token = random_token("sess");
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.chars().take(256).collect::<String>());
    sqlx::query(
        "INSERT INTO sessions(user_id,token_hash,expires_at,user_agent,last_seen_at) \
         VALUES ($1,$2,now()+interval '7 days',$3,now())",
    )
    .bind(user_id)
    .bind(hash_token(&token))
    .bind(user_agent)
    .execute(db)
    .await?;
    Ok(token)
}

/// Verifies a password off the async runtime. Unknown users are checked
/// against a fixed hash so the response time does not reveal registration.
pub async fn verify_password_blocking(password: String, encoded: Option<String>) -> Result<bool> {
    static UNKNOWN_USER_HASH: LazyLock<String> =
        LazyLock::new(|| hash_password("unknown-user-timing-placeholder").unwrap_or_default());
    tokio::task::spawn_blocking(move || {
        let known = encoded.is_some();
        let encoded = encoded.unwrap_or_else(|| UNKNOWN_USER_HASH.clone());
        verify_password(&password, &encoded) && known
    })
    .await
    .map_err(|_| AppError::Internal("password check failed".into()))
}

/// The proxy replaces untrusted forwarding headers, so its last entry is the client.
pub fn client_ip(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.rsplit(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 64)
        .map(ToOwned::to_owned)
}

const LOGIN_WINDOW_SECONDS: i64 = 900;

fn login_failure_keys(email: &str, ip: Option<&str>) -> Vec<(String, i64)> {
    let mut keys = vec![(
        format!("meta-router:login-failures:email:{}", hash_token(email)),
        10,
    )];
    if let Some(ip) = ip {
        keys.push((
            format!("meta-router:login-failures:ip:{}", hash_token(ip)),
            50,
        ));
    }
    keys
}

/// Failed sign-ins are limited per email and per client address. A Redis
/// outage fails open so administrators can still reach the console.
pub async fn ensure_login_allowed(state: &AppState, email: &str, ip: Option<&str>) -> Result<()> {
    let Ok(mut redis) = state.redis.get_multiplexed_async_connection().await else {
        eprintln!("login attempt limiter unavailable");
        return Ok(());
    };
    for (key, maximum) in login_failure_keys(email, ip) {
        let count: Option<i64> = redis::cmd("GET")
            .arg(&key)
            .query_async(&mut redis)
            .await
            .unwrap_or(None);
        if count.unwrap_or(0) >= maximum {
            return Err(AppError::TooManyAttempts(
                "登录失败次数过多，请 15 分钟后再试".into(),
            ));
        }
    }
    Ok(())
}

pub async fn record_login_failure(state: &AppState, email: &str, ip: Option<&str>) {
    let Ok(mut redis) = state.redis.get_multiplexed_async_connection().await else {
        return;
    };
    for (key, _) in login_failure_keys(email, ip) {
        let _: redis::RedisResult<i64> = redis::Script::new(
            "local n = redis.call('INCR', KEYS[1]); if n == 1 then redis.call('EXPIRE', KEYS[1], ARGV[1]) end; return n",
        )
        .key(&key)
        .arg(LOGIN_WINDOW_SECONDS)
        .invoke_async(&mut redis)
        .await;
    }
}

/// A successful sign-in or password reset proves ownership of the email.
pub async fn clear_login_failures(state: &AppState, email: &str) {
    if let Ok(mut redis) = state.redis.get_multiplexed_async_connection().await {
        let _: redis::RedisResult<i64> = redis::cmd("DEL")
            .arg(login_failure_keys(email, None)[0].0.as_str())
            .query_async(&mut redis)
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(axum::http::header::HOST, value.parse().unwrap());
        headers
    }

    #[test]
    fn secure_cookies_are_skipped_only_on_local_origins() {
        assert!(cookie_secure(true, &host("gateway.example.com")));
        assert!(cookie_secure(true, &host("10.0.0.5:8080")));
        assert!(cookie_secure(true, &HeaderMap::new()));
        assert!(!cookie_secure(true, &host("localhost:8080")));
        assert!(!cookie_secure(true, &host("127.0.0.1:8080")));
        assert!(!cookie_secure(true, &host("[::1]:8080")));
        assert!(!cookie_secure(true, &host("app.localhost")));
        assert!(!cookie_secure(false, &host("gateway.example.com")));
    }
}
