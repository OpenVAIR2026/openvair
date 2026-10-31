//! VAIR Desktop sign-in and the narrow API a signed-in client may use.
//!
//! Sign-in follows the OAuth device authorization grant (RFC 8628): the app
//! asks for a device code, shows the short user code, and polls while the user
//! approves the code in a browser that is already signed in. The resulting
//! client token can list the account's keys and reveal one, nothing else.
use crate::{
    audit, auth,
    error::{AppError, Result},
    key_views, reveal_key_secret,
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header::CACHE_CONTROL},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use rand::{Rng, rngs::OsRng};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::FromRow;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[cfg(test)]
#[path = "client_api_db_tests.rs"]
mod db_tests;

const DEVICE_CODE_SECONDS: i64 = 600;
const POLL_INTERVAL_SECONDS: i64 = 5;
const AUTHORIZE_WINDOW_SECONDS: i64 = 900;
const AUTHORIZE_LIMIT_PER_IP: i64 = 20;
/// No vowels, so codes never spell words, and no 0/O or 1/I look-alikes.
const USER_CODE_ALPHABET: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ23456789";
pub const ACTIVATE_PATH: &str = "/client/activate";

fn new_user_code() -> String {
    let mut rng = OsRng;
    (0..8)
        .map(|_| USER_CODE_ALPHABET[rng.gen_range(0..USER_CODE_ALPHABET.len())] as char)
        .collect()
}

/// Accepts what a person types: any case, with or without the dash.
fn normalize_user_code(value: &str) -> Result<String> {
    let code: String = value
        .chars()
        .filter(|c| !matches!(c, '-' | ' '))
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if code.len() == 8 && code.bytes().all(|b| USER_CODE_ALPHABET.contains(&b)) {
        Ok(code)
    } else {
        Err(AppError::NotFound)
    }
}

fn display_user_code(code: &str) -> String {
    format!("{}-{}", &code[..4], &code[4..])
}

fn validate_client_name(value: &str) -> Result<String> {
    let name = value.trim();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(AppError::BadRequest(
            "客户端名称不能为空，且不能超过 120 个字符".into(),
        ));
    }
    Ok(name.to_owned())
}

/// Device codes are cheap to request, so each address gets a budget. A Redis
/// outage fails open, like the sign-in limiter.
async fn limit_authorize(state: &AppState, headers: &HeaderMap) -> Result<()> {
    let Some(ip) = auth::client_ip(headers) else {
        return Ok(());
    };
    let Ok(mut redis) = state.redis.get_multiplexed_async_connection().await else {
        return Ok(());
    };
    let key = format!("meta-router:client-authorize:ip:{}", auth::hash_token(&ip));
    let count: i64 = redis::Script::new(
        "local n = redis.call('INCR', KEYS[1]); if n == 1 then redis.call('EXPIRE', KEYS[1], ARGV[1]) end; return n",
    )
    .key(&key)
    .arg(AUTHORIZE_WINDOW_SECONDS)
    .invoke_async(&mut redis)
    .await
    .unwrap_or(0);
    if count > AUTHORIZE_LIMIT_PER_IP {
        return Err(AppError::TooManyAttempts("请求过于频繁，请稍后再试".into()));
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct AuthorizeInput {
    client_name: String,
}

pub async fn authorize(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<AuthorizeInput>,
) -> Result<Json<Value>> {
    let client_name = validate_client_name(&input.client_name)?;
    limit_authorize(&state, &headers).await?;
    let device_code = auth::random_token("vdc");
    // A collision on the short code is possible but rare; retry a few times.
    for _ in 0..5 {
        let user_code = new_user_code();
        let inserted = sqlx::query(
            "INSERT INTO client_device_authorizations(device_code_hash,user_code,client_name,expires_at)
             VALUES ($1,$2,$3,now()+make_interval(secs => $4))",
        )
        .bind(auth::hash_token(&device_code))
        .bind(&user_code)
        .bind(&client_name)
        .bind(DEVICE_CODE_SECONDS as f64)
        .execute(&state.db)
        .await;
        match inserted {
            Ok(_) => {
                let display = display_user_code(&user_code);
                return Ok(Json(json!({
                    "device_code": device_code,
                    "user_code": display,
                    "verification_path": ACTIVATE_PATH,
                    "verification_path_complete": format!("{ACTIVATE_PATH}?code={display}"),
                    "expires_in": DEVICE_CODE_SECONDS,
                    "interval": POLL_INTERVAL_SECONDS,
                })));
            }
            Err(error)
                if error
                    .as_database_error()
                    .is_some_and(|error| error.is_unique_violation()) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(AppError::Internal("could not allocate a user code".into()))
}

/// Token endpoint errors use the RFC 8628 shape the client polls on.
fn grant_error(code: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": code }))).into_response()
}

#[derive(Deserialize)]
pub struct TokenInput {
    device_code: String,
}

#[derive(FromRow)]
struct PendingGrant {
    id: Uuid,
    status: String,
    user_id: Option<Uuid>,
    client_name: String,
    expired: bool,
    too_fast: bool,
}

pub async fn token(
    State(state): State<AppState>,
    Json(input): Json<TokenInput>,
) -> Result<Response> {
    let mut tx = state.db.begin().await?;
    let Some(grant) = sqlx::query_as::<_, PendingGrant>(
        "SELECT id,status,user_id,client_name,expires_at<=now() AS expired,
                COALESCE(last_polled_at > now()-make_interval(secs => $2), false) AS too_fast
         FROM client_device_authorizations WHERE device_code_hash=$1 FOR UPDATE",
    )
    .bind(auth::hash_token(&input.device_code))
    .bind(POLL_INTERVAL_SECONDS as f64)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(grant_error("invalid_grant"));
    };
    if grant.expired {
        return Ok(grant_error("expired_token"));
    }
    match grant.status.as_str() {
        "pending" => {
            sqlx::query("UPDATE client_device_authorizations SET last_polled_at=now() WHERE id=$1")
                .bind(grant.id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            Ok(grant_error(if grant.too_fast {
                "slow_down"
            } else {
                "authorization_pending"
            }))
        }
        "denied" => Ok(grant_error("access_denied")),
        "approved" => {
            let user_id = grant
                .user_id
                .ok_or_else(|| AppError::Internal("approved grant without user".into()))?;
            let token = auth::random_token("vcl");
            let (token_id, expires_at): (Uuid, DateTime<Utc>) = sqlx::query_as(
                "INSERT INTO client_tokens(user_id,token_hash,client_name,expires_at)
                 VALUES ($1,$2,$3,now()+interval '30 days') RETURNING id,expires_at",
            )
            .bind(user_id)
            .bind(auth::hash_token(&token))
            .bind(&grant.client_name)
            .fetch_one(&mut *tx)
            .await?;
            sqlx::query("UPDATE client_device_authorizations SET status='consumed' WHERE id=$1")
                .bind(grant.id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            Ok((
                [(CACHE_CONTROL, "no-store")],
                Json(json!({
                    "access_token": token,
                    "token_type": "Bearer",
                    "token_id": token_id,
                    "expires_at": expires_at,
                })),
            )
                .into_response())
        }
        _ => Ok(grant_error("invalid_grant")),
    }
}

#[derive(Serialize, FromRow)]
pub struct DeviceRequestView {
    client_name: String,
    #[sqlx(skip)]
    user_code: String,
    status: String,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

/// What the browser shows before the user approves a code.
pub async fn show(
    State(state): State<AppState>,
    Path(code): Path<String>,
    headers: HeaderMap,
) -> Result<Json<DeviceRequestView>> {
    let user = auth::current_user(&state, &headers).await?;
    let code = normalize_user_code(&code)?;
    let mut view = sqlx::query_as::<_, DeviceRequestView>(
        "SELECT client_name,status,created_at,expires_at FROM client_device_authorizations
         WHERE user_code=$1 AND expires_at>now() AND (status='pending' OR user_id=$2)",
    )
    .bind(&code)
    .bind(user.user_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    view.user_code = display_user_code(&code);
    Ok(Json(view))
}

async fn decide(state: &AppState, headers: &HeaderMap, code: &str, status: &str) -> Result<()> {
    let user = auth::current_user(state, headers).await?;
    let code = normalize_user_code(code)?;
    let id: Uuid = sqlx::query_scalar(
        "UPDATE client_device_authorizations SET status=$3,user_id=$2
         WHERE user_code=$1 AND status='pending' AND expires_at>now() RETURNING id",
    )
    .bind(&code)
    .bind(user.user_id)
    .bind(status)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    let action = if status == "approved" {
        "client.approved"
    } else {
        "client.denied"
    };
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        action,
        "client_device",
        id,
        json!({}),
    )
    .await;
    Ok(())
}

pub async fn approve(
    State(state): State<AppState>,
    Path(code): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    decide(&state, &headers, &code, "approved").await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn deny(
    State(state): State<AppState>,
    Path(code): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    decide(&state, &headers, &code, "denied").await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn me(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let client = auth::current_client(&state, &headers).await?;
    let account_name: String = sqlx::query_scalar("SELECT name FROM accounts WHERE id=$1")
        .bind(client.user.account_id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(json!({
        "email": client.user.email,
        "account_name": account_name,
        "client_name": client.client_name,
        "expires_at": client.expires_at,
    })))
}

/// A key as the desktop client sees it: enough to pick one, never the secret.
#[derive(Serialize)]
pub struct ClientKeyView {
    id: Uuid,
    name: String,
    key_prefix: String,
    enabled: bool,
    can_copy: bool,
    used_cost_today_usd: f64,
    cost_limit_daily_usd: Option<f64>,
    /// Routable models by protocol, including those reached through VAIR key
    /// channels; empty for a disabled key.
    models: BTreeMap<String, BTreeSet<String>>,
    /// The key group, so the client can fold keys by group.
    group: Option<ClientKeyGroup>,
}

#[derive(Serialize, FromRow)]
pub struct ClientKeyGroup {
    id: Uuid,
    name: String,
}

pub async fn keys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ClientKeyView>>> {
    let client = auth::current_client(&state, &headers).await?;
    let groups: Vec<ClientKeyGroup> =
        sqlx::query_as("SELECT id,name FROM key_groups WHERE account_id=$1")
            .bind(client.user.account_id)
            .fetch_all(&state.db)
            .await?;
    let mut result = Vec::new();
    for key in key_views(&state, client.user.account_id).await? {
        let group = key
            .group_id
            .and_then(|id| groups.iter().find(|group| group.id == id))
            .map(|group| ClientKeyGroup {
                id: group.id,
                name: group.name.clone(),
            });
        result.push(ClientKeyView {
            id: key.id,
            name: key.name,
            key_prefix: key.key_prefix,
            enabled: key.enabled,
            can_copy: key.can_copy,
            used_cost_today_usd: key.used_cost_today_usd,
            cost_limit_daily_usd: key.cost_limit_daily_usd,
            models: key.models,
            group,
        });
    }
    Ok(Json(result))
}

pub async fn reveal(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response> {
    let client = auth::current_client(&state, &headers).await?;
    let enabled: bool = sqlx::query_scalar(
        "SELECT enabled FROM virtual_api_keys WHERE id=$1 AND account_id=$2 AND revoked_at IS NULL",
    )
    .bind(id)
    .bind(client.user.account_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    if !enabled {
        return Err(AppError::Conflict(
            "此 Key 已停用，请在控制台启用或选择其他 Key".into(),
        ));
    }
    let raw = reveal_key_secret(
        &state,
        &client.user,
        id,
        json!({"via":"client","client_token_id":client.token_id}),
    )
    .await?;
    Ok(([(CACHE_CONTROL, "no-store")], Json(json!({"key":raw}))).into_response())
}

/// Signs this client out. Keys already written to agents keep working.
pub async fn sign_out(State(state): State<AppState>, headers: HeaderMap) -> Result<StatusCode> {
    let client = auth::current_client(&state, &headers).await?;
    sqlx::query("UPDATE client_tokens SET revoked_at=now() WHERE id=$1")
        .bind(client.token_id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, FromRow)]
pub struct ClientTokenView {
    id: Uuid,
    client_name: String,
    created_at: DateTime<Utc>,
    last_seen_at: Option<DateTime<Utc>>,
    expires_at: DateTime<Utc>,
}

/// Authorized desktop clients, for the console's settings page.
pub async fn list_tokens(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ClientTokenView>>> {
    let user = auth::current_user(&state, &headers).await?;
    Ok(Json(
        sqlx::query_as(
            "SELECT id,client_name,created_at,last_seen_at,expires_at FROM client_tokens
             WHERE user_id=$1 AND revoked_at IS NULL AND expires_at>now()
             ORDER BY COALESCE(last_seen_at,created_at) DESC, id",
        )
        .bind(user.user_id)
        .fetch_all(&state.db)
        .await?,
    ))
}

pub async fn revoke_token(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let user = auth::current_user(&state, &headers).await?;
    let revoked = sqlx::query(
        "UPDATE client_tokens SET revoked_at=now() WHERE id=$1 AND user_id=$2 AND revoked_at IS NULL",
    )
    .bind(id)
    .bind(user.user_id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if revoked == 0 {
        return Err(AppError::NotFound);
    }
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "client.revoked",
        "client_token",
        id,
        json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_codes_normalize_and_display() {
        let code = new_user_code();
        assert_eq!(code.len(), 8);
        let typed = display_user_code(&code).to_ascii_lowercase();
        assert_eq!(normalize_user_code(&typed).unwrap(), code);
        assert!(normalize_user_code("ABCD-EFGH").is_err());
        assert!(normalize_user_code("BCDF").is_err());
    }
}
