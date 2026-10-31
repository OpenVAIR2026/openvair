//! One-time links that hand a key to another account as a VAIR key channel.
//! The link replaces passing the secret around: the recipient adds the channel
//! without ever seeing the key. Once accepted the channel stands on
//! `source_key_id` alone, like a pasted key, so the link takes no part in
//! routing and revoking the key remains the way to take it back.
use crate::{
    auth,
    error::{AppError, Result},
    key_holder, load_account_limits, normalize_model_mapping,
    state::AppState,
    vair_key_channels::{self, NewChannel, SourceKeyView},
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

#[cfg(test)]
#[path = "key_share_links_db_tests.rs"]
mod db_tests;

/// Every reason a code cannot be used gets the same answer.
const INVALID_LINK: &str = "分享链接无效或已过期";
const CODE_PREFIX: &str = "vair_share";
const FAILURE_SCOPE: &str = "key-share";
const FAILURE_LIMIT_PER_IP: i64 = 30;

#[derive(Serialize)]
pub struct CreatedLink {
    code: String,
    expires_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct ResolveInput {
    code: String,
}

#[derive(Serialize)]
pub struct LinkView {
    #[serde(flatten)]
    source: SourceKeyView,
    expires_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct AcceptInput {
    code: String,
    name: String,
    priority: Option<i32>,
    model_mapping: Option<Value>,
}

#[derive(sqlx::FromRow)]
struct OpenLink {
    id: Uuid,
    virtual_key_id: Uuid,
    account_id: Uuid,
    created_by: Option<Uuid>,
    expires_at: DateTime<Utc>,
}

const OPEN_LINK: &str = "SELECT id,virtual_key_id,account_id,created_by,expires_at
    FROM key_share_links
    WHERE code_hash=$1 AND accepted_at IS NULL AND cancelled_at IS NULL AND expires_at>now()";

async fn audit(
    db: &PgPool,
    account_id: Uuid,
    user_id: Option<Uuid>,
    action: &str,
    resource_type: &str,
    resource_id: Uuid,
    metadata: Value,
) {
    let _ = sqlx::query(
        "INSERT INTO audit_logs(account_id,user_id,action,resource_type,resource_id,metadata)
         VALUES ($1,$2,$3,$4,$5,$6)",
    )
    .bind(account_id)
    .bind(user_id)
    .bind(action)
    .bind(resource_type)
    .bind(resource_id)
    .bind(metadata)
    .execute(db)
    .await;
}

/// Codes are long random secrets, so guessing is hopeless; the budget only
/// keeps one address from hammering the lookup.
async fn ensure_attempts_left(state: &AppState, headers: &HeaderMap) -> Result<()> {
    if key_holder::failure_count(state, headers, FAILURE_SCOPE).await >= FAILURE_LIMIT_PER_IP {
        return Err(AppError::TooManyAttempts("尝试次数过多，请稍后再试".into()));
    }
    Ok(())
}

async fn invalid(state: &AppState, headers: &HeaderMap) -> AppError {
    key_holder::record_failure(state, headers, FAILURE_SCOPE).await;
    AppError::BadRequest(INVALID_LINK.into())
}

fn code_hash(code: &str) -> Option<String> {
    let code = code.trim();
    (code.starts_with(CODE_PREFIX) && code.len() <= 128).then(|| auth::hash_token(code))
}

/// The usable source key behind a link, refusing the sharer's own account.
async fn link_source(state: &AppState, link: &OpenLink, account_id: Uuid) -> Result<SourceKeyView> {
    let source = vair_key_channels::source_by_id(state, link.virtual_key_id, account_id)
        .await?
        .ok_or_else(|| AppError::BadRequest(INVALID_LINK.into()))?;
    vair_key_channels::ensure_foreign(&source, account_id)?;
    Ok(source)
}

/// POST /api/keys/{id}/share — issue a link for one of the account's keys,
/// cancelling any link still open for it.
pub async fn create(
    State(state): State<AppState>,
    Path(key_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<CreatedLink>> {
    let user = auth::current_user(&state, &headers).await?;
    let mut tx = state.db.begin().await?;
    // The row lock serializes concurrent shares of the same key.
    let enabled: bool = sqlx::query_scalar(
        "SELECT enabled FROM virtual_api_keys
         WHERE id=$1 AND account_id=$2 AND revoked_at IS NULL FOR UPDATE",
    )
    .bind(key_id)
    .bind(user.account_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    if !enabled {
        return Err(AppError::BadRequest("请先启用这把 Key 再分享".into()));
    }
    sqlx::query(
        "UPDATE key_share_links SET cancelled_at=now()
         WHERE virtual_key_id=$1 AND accepted_at IS NULL AND cancelled_at IS NULL",
    )
    .bind(key_id)
    .execute(&mut *tx)
    .await?;
    let code = auth::random_token(CODE_PREFIX);
    let (link_id, expires_at): (Uuid, DateTime<Utc>) = sqlx::query_as(
        "INSERT INTO key_share_links(virtual_key_id,account_id,code_hash,created_by,expires_at)
         VALUES ($1,$2,$3,$4,now()+interval '24 hours') RETURNING id,expires_at",
    )
    .bind(key_id)
    .bind(user.account_id)
    .bind(auth::hash_token(&code))
    .bind(user.user_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    audit(
        &state.db,
        user.account_id,
        Some(user.user_id),
        "key_share.created",
        "virtual_key",
        key_id,
        json!({"link_id": link_id, "expires_at": expires_at}),
    )
    .await;
    Ok(Json(CreatedLink { code, expires_at }))
}

/// DELETE /api/keys/{id}/share — cancel the key's open link.
pub async fn cancel(
    State(state): State<AppState>,
    Path(key_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let user = auth::current_user(&state, &headers).await?;
    let cancelled = sqlx::query(
        "UPDATE key_share_links SET cancelled_at=now()
         WHERE virtual_key_id=$1 AND account_id=$2 AND accepted_at IS NULL AND cancelled_at IS NULL",
    )
    .bind(key_id)
    .bind(user.account_id)
    .execute(&state.db)
    .await?;
    if cancelled.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    audit(
        &state.db,
        user.account_id,
        Some(user.user_id),
        "key_share.cancelled",
        "virtual_key",
        key_id,
        json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/key-shares/resolve — preview a link before accepting it. The code
/// travels in the body so it stays out of URLs and access logs.
pub async fn resolve(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<ResolveInput>,
) -> Result<Json<LinkView>> {
    let user = auth::current_user(&state, &headers).await?;
    ensure_attempts_left(&state, &headers).await?;
    let Some(hash) = code_hash(&input.code) else {
        return Err(invalid(&state, &headers).await);
    };
    let Some(link) = sqlx::query_as::<_, OpenLink>(OPEN_LINK)
        .bind(hash)
        .fetch_optional(&state.db)
        .await?
    else {
        return Err(invalid(&state, &headers).await);
    };
    let source = link_source(&state, &link, user.account_id).await?;
    Ok(Json(LinkView {
        source,
        expires_at: link.expires_at,
    }))
}

/// POST /api/key-shares/accept — use the link once to add its key as a channel.
/// A failure such as a full channel list leaves the link open for a retry.
pub async fn accept(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<AcceptInput>,
) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    let name = vair_key_channels::normalize_name(&input.name)?;
    let priority = input.priority.unwrap_or(100);
    if priority < 0 {
        return Err(AppError::BadRequest("priority must be non-negative".into()));
    }
    let mapping = normalize_model_mapping(input.model_mapping.unwrap_or_else(|| json!({})))?;
    ensure_attempts_left(&state, &headers).await?;
    let Some(hash) = code_hash(&input.code) else {
        return Err(invalid(&state, &headers).await);
    };
    let channel_limit = load_account_limits(&state.db).await.channel_limit;
    let mut tx = state.db.begin().await?;
    // Concurrent accepts queue on the row; the loser re-reads it as accepted.
    let link = sqlx::query_as::<_, OpenLink>(&format!("{OPEN_LINK} FOR UPDATE"))
        .bind(hash)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(link) = link else {
        drop(tx);
        return Err(invalid(&state, &headers).await);
    };
    let source = link_source(&state, &link, user.account_id).await?;
    let channel_id = vair_key_channels::insert_channel(
        &mut tx,
        channel_limit,
        NewChannel {
            account_id: user.account_id,
            name: &name,
            priority,
            model_mapping: &mapping,
            source_key_id: source.id,
        },
    )
    .await?;
    sqlx::query("UPDATE key_share_links SET accepted_at=now(),accepted_account_id=$2 WHERE id=$1")
        .bind(link.id)
        .bind(user.account_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    audit(
        &state.db,
        user.account_id,
        Some(user.user_id),
        "channel.created",
        "channel",
        channel_id,
        json!({"kind": "vair_key", "via": "share_link"}),
    )
    .await;
    audit(
        &state.db,
        link.account_id,
        link.created_by,
        "key_share.accepted",
        "virtual_key",
        link.virtual_key_id,
        json!({"link_id": link.id}),
    )
    .await;
    Ok(Json(json!({"id": channel_id})))
}
