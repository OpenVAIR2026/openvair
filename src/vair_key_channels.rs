//! Keys from other accounts used as channels. A channel stores only the source
//! key id: the gateway re-checks that key on every request, so the recipient
//! never sees the owner's upstream credentials. There is no invite or approval
//! step, because the only thing that can add such a channel is being handed the
//! key itself.
use crate::{
    audit, auth,
    error::{AppError, Result},
    gateway, load_account_limits, normalize_model_mapping, reserve_channel_slot,
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, PgConnection, PgPool};
use uuid::Uuid;

#[cfg(test)]
#[path = "vair_key_channels_db_tests.rs"]
mod db_tests;

const INVALID_KEY: &str = "Key 无效或已撤销";

/// What the recipient may see about a source key: its owner account, label and
/// limits, never the upstream channels behind it.
#[derive(Serialize, FromRow)]
pub struct SourceKeyView {
    #[serde(skip)]
    pub id: Uuid,
    #[serde(skip)]
    pub account_id: Uuid,
    account_name: String,
    key_name: String,
    key_prefix: String,
    allowed_models: Value,
    model_mode: String,
    requests_per_minute: Option<i32>,
    token_limit_daily: Option<i64>,
    cost_limit_daily_usd: Option<f64>,
    cost_limit_total_usd: Option<f64>,
    concurrency_limit: i32,
    #[sqlx(skip)]
    models: Vec<String>,
}

const SOURCE_KEY_COLUMNS: &str = "v.id,v.account_id,a.name AS account_name,v.name AS key_name,
    v.key_prefix,v.allowed_models,v.model_mode,v.requests_per_minute,v.token_limit_daily,
    v.cost_limit_daily_usd,v.cost_limit_total_usd,v.concurrency_limit";

#[derive(Deserialize)]
pub struct ResolveInput {
    secret: String,
}

#[derive(Deserialize)]
pub struct CreateChannelInput {
    name: String,
    secret: String,
    priority: Option<i32>,
    model_mapping: Option<Value>,
}

/// Rejects operations that only make sense for channels with an upstream URL
/// and credential, such as health checks and model sync.
pub async fn ensure_upstream_channel(
    db: &PgPool,
    account_id: Uuid,
    channel_id: Uuid,
) -> Result<()> {
    ensure_channel_kind(db, account_id, channel_id, "upstream").await
}

async fn ensure_channel_kind(
    db: &PgPool,
    account_id: Uuid,
    channel_id: Uuid,
    expected: &str,
) -> Result<()> {
    let kind: Option<String> =
        sqlx::query_scalar("SELECT kind FROM channels WHERE id=$1 AND account_id=$2")
            .bind(channel_id)
            .bind(account_id)
            .fetch_optional(db)
            .await?;
    let refused = match expected {
        "vair_key" => "只有 VAIR Key 渠道支持此操作",
        _ => "VAIR Key 渠道不支持此操作",
    };
    match kind.as_deref() {
        None => Err(AppError::NotFound),
        Some(kind) if kind == expected => Ok(()),
        Some(_) => Err(AppError::BadRequest(refused.into())),
    }
}

async fn source_by_secret(state: &AppState, secret: &str, viewer: Uuid) -> Result<SourceKeyView> {
    let secret = secret.trim();
    if secret.is_empty() || secret.len() > 512 {
        return Err(AppError::BadRequest(INVALID_KEY.into()));
    }
    let id: Uuid = sqlx::query_scalar("SELECT id FROM virtual_api_keys WHERE key_hash=$1")
        .bind(auth::hash_token(secret))
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::BadRequest(INVALID_KEY.into()))?;
    source_by_id(state, id, viewer)
        .await?
        .ok_or_else(|| AppError::BadRequest(INVALID_KEY.into()))
}

/// The source key if it can still serve requests: enabled, not revoked and
/// owned by an account that is not disabled, with the models `viewer` would
/// reach through it.
pub(crate) async fn source_by_id(
    state: &AppState,
    id: Uuid,
    viewer: Uuid,
) -> Result<Option<SourceKeyView>> {
    let source = sqlx::query_as::<_, SourceKeyView>(&format!(
        "SELECT {SOURCE_KEY_COLUMNS} FROM virtual_api_keys v JOIN accounts a ON a.id=v.account_id
         WHERE v.id=$1 AND v.enabled=true AND v.revoked_at IS NULL AND a.disabled_at IS NULL"
    ))
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let Some(mut source) = source else {
        return Ok(None);
    };
    source.models = gateway::source_key_model_names(state, source.id, viewer).await?;
    Ok(Some(source))
}

pub(crate) fn ensure_foreign(source: &SourceKeyView, account_id: Uuid) -> Result<()> {
    if source.account_id == account_id {
        return Err(AppError::BadRequest("不能把本帐号的 Key 添加为渠道".into()));
    }
    Ok(())
}

pub(crate) fn normalize_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(AppError::BadRequest("channel name is required".into()));
    }
    Ok(name.to_owned())
}

pub(crate) struct NewChannel<'a> {
    pub account_id: Uuid,
    pub name: &'a str,
    pub priority: i32,
    pub model_mapping: &'a Value,
    pub source_key_id: Uuid,
}

pub(crate) async fn insert_channel(
    conn: &mut PgConnection,
    channel_limit: i64,
    channel: NewChannel<'_>,
) -> Result<Uuid> {
    reserve_channel_slot(&mut *conn, channel.account_id, channel_limit).await?;
    let id: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO channels(account_id,name,protocol,base_url,priority,models,model_mapping,
             model_mode,kind,source_key_id)
         VALUES ($1,$2,'any','',$3,'[]'::jsonb,$4,'passthrough','vair_key',$5)
         ON CONFLICT (account_id,source_key_id) WHERE source_key_id IS NOT NULL DO NOTHING
         RETURNING id",
    )
    .bind(channel.account_id)
    .bind(channel.name)
    .bind(channel.priority)
    .bind(channel.model_mapping)
    .bind(channel.source_key_id)
    .fetch_optional(&mut *conn)
    .await?;
    let id = id.ok_or_else(|| AppError::Conflict("这把 Key 已经是本帐号的渠道".into()))?;
    // Scheduled checks spend the source key's quota, so the recipient opts in.
    sqlx::query(
        "INSERT INTO channel_monitor_configs(channel_id,availability_enabled) VALUES ($1,false)",
    )
    .bind(id)
    .execute(&mut *conn)
    .await?;
    Ok(id)
}

/// POST /api/channels/vair-key/resolve — preview a pasted key before adding it.
pub async fn resolve(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<ResolveInput>,
) -> Result<Json<SourceKeyView>> {
    let user = auth::current_user(&state, &headers).await?;
    let source = source_by_secret(&state, &input.secret, user.account_id).await?;
    ensure_foreign(&source, user.account_id)?;
    Ok(Json(source))
}

/// POST /api/channels/vair-key — add a pasted key from another account as a channel.
pub async fn create_channel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateChannelInput>,
) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    let name = normalize_name(&input.name)?;
    let priority = input.priority.unwrap_or(100);
    if priority < 0 {
        return Err(AppError::BadRequest("priority must be non-negative".into()));
    }
    let mapping = normalize_model_mapping(input.model_mapping.unwrap_or_else(|| json!({})))?;
    let source = source_by_secret(&state, &input.secret, user.account_id).await?;
    ensure_foreign(&source, user.account_id)?;
    let channel_limit = load_account_limits(&state.db).await.channel_limit;
    let mut tx = state.db.begin().await?;
    let channel_id = insert_channel(
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
    tx.commit().await?;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "channel.created",
        "channel",
        channel_id,
        json!({"kind": "vair_key", "via": "key"}),
    )
    .await;
    Ok(Json(json!({"id": channel_id})))
}

/// POST /api/channels/{id}/vair-key — point a VAIR key channel at another key,
/// keeping its name, priority, mappings and monitors.
pub async fn replace_key(
    State(state): State<AppState>,
    Path(channel_id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<ResolveInput>,
) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    ensure_channel_kind(&state.db, user.account_id, channel_id, "vair_key").await?;
    let source = source_by_secret(&state, &input.secret, user.account_id).await?;
    ensure_foreign(&source, user.account_id)?;
    let mut tx = state.db.begin().await?;
    // The (account_id, source_key_id) unique index rejects a key that another
    // channel of this account already uses, including one added concurrently.
    let updated = sqlx::query(
        "UPDATE channels SET source_key_id=$3,updated_at=now()
         WHERE id=$1 AND account_id=$2 AND kind='vair_key'",
    )
    .bind(channel_id)
    .bind(user.account_id)
    .bind(source.id)
    .execute(&mut *tx)
    .await
    .map_err(|error| {
        if error
            .as_database_error()
            .is_some_and(|error| error.is_unique_violation())
        {
            AppError::Conflict("这把 Key 已经是本帐号的另一个渠道".into())
        } else {
            error.into()
        }
    })?;
    if updated.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    // Check results describe the previous key.
    sqlx::query("DELETE FROM channel_health WHERE channel_id=$1")
        .bind(channel_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "channel.updated",
        "channel",
        channel_id,
        json!({"source_key_replaced": true}),
    )
    .await;
    Ok(Json(json!({"id": channel_id})))
}
