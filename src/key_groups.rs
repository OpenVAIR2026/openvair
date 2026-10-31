//! Key groups give several keys one channel and model scope, so changing the
//! scope of a class of keys is one edit. The group's scope is copied onto each
//! member key in the same transaction as the change, which keeps the gateway
//! and every other reader on the key's own columns. Limits stay per key.
use crate::{
    audit, auth,
    error::{AppError, Result},
    key_channels, model_rules, normalize_allowed_models,
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, PgConnection};
use uuid::Uuid;

#[cfg(test)]
#[path = "key_groups_db_tests.rs"]
mod db_tests;

#[derive(Debug, Serialize, FromRow)]
pub struct KeyGroupView {
    id: Uuid,
    name: String,
    channel_mode: String,
    channel_ids: Vec<Uuid>,
    model_mode: String,
    allowed_models: Value,
    /// Unrevoked keys in the group.
    key_count: i64,
}

#[derive(Debug, Deserialize)]
pub struct KeyGroupInput {
    name: String,
    channel_mode: String,
    #[serde(default)]
    channel_ids: Vec<Uuid>,
    model_mode: String,
    #[serde(default)]
    allowed_models: Vec<String>,
    /// The group's exact members; omitted keeps the current ones.
    key_ids: Option<Vec<Uuid>>,
}

struct Scope {
    name: String,
    channel_mode: String,
    channel_ids: Vec<Uuid>,
    model_mode: String,
    allowed_models: Vec<String>,
    key_ids: Option<Vec<Uuid>>,
}

async fn validate(state: &AppState, account_id: Uuid, input: KeyGroupInput) -> Result<Scope> {
    let name = input.name.trim().to_owned();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(AppError::BadRequest(
            "分组名称不能为空，最长 120 个字符".into(),
        ));
    }
    if !matches!(input.model_mode.as_str(), "all" | "allowlist") {
        return Err(AppError::BadRequest("invalid model mode".into()));
    }
    let allowed_models = normalize_allowed_models(input.allowed_models)?;
    model_rules::validate_mode(&input.model_mode, "all", &allowed_models)?;
    let allowed_models = model_rules::key_rules(&input.model_mode, allowed_models);
    let channel_ids = key_channels(
        &state.db,
        account_id,
        &input.channel_mode,
        input.channel_ids,
    )
    .await?;
    Ok(Scope {
        name,
        channel_mode: input.channel_mode,
        channel_ids,
        model_mode: input.model_mode,
        allowed_models,
        key_ids: input.key_ids,
    })
}

fn name_taken(error: sqlx::Error) -> AppError {
    if error
        .as_database_error()
        .is_some_and(|error| error.is_unique_violation())
    {
        AppError::Conflict("分组名称已存在".into())
    } else {
        error.into()
    }
}

/// Copies the group's scope onto its keys, or onto `key_id` alone.
pub(crate) async fn copy_scope(
    conn: &mut PgConnection,
    group_id: Uuid,
    key_id: Option<Uuid>,
) -> Result<()> {
    sqlx::query(
        "UPDATE virtual_api_keys v SET channel_mode=g.channel_mode,model_mode=g.model_mode,allowed_models=g.allowed_models
         FROM key_groups g WHERE g.id=$1 AND v.group_id=g.id AND ($2::uuid IS NULL OR v.id=$2)",
    )
    .bind(group_id)
    .bind(key_id)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "DELETE FROM virtual_key_channels WHERE virtual_key_id IN
         (SELECT id FROM virtual_api_keys WHERE group_id=$1 AND ($2::uuid IS NULL OR id=$2))",
    )
    .bind(group_id)
    .bind(key_id)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO virtual_key_channels(virtual_key_id,channel_id)
         SELECT v.id,gc.channel_id FROM virtual_api_keys v JOIN key_group_channels gc ON gc.group_id=v.group_id
         WHERE v.group_id=$1 AND ($2::uuid IS NULL OR v.id=$2)",
    )
    .bind(group_id)
    .bind(key_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Locks the account's group against concurrent scope changes while a key joins it.
pub(crate) async fn lock_group(
    conn: &mut PgConnection,
    account_id: Uuid,
    group_id: Uuid,
) -> Result<()> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM key_groups WHERE id=$1 AND account_id=$2 FOR SHARE",
    )
    .bind(group_id)
    .bind(account_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| AppError::BadRequest("分组不存在".into()))?;
    Ok(())
}

/// Makes `ids` the group's unrevoked members. Keys from other groups move in;
/// keys left out stay ungrouped with the scope they carry.
async fn set_members(
    conn: &mut PgConnection,
    account_id: Uuid,
    group_id: Uuid,
    ids: &[Uuid],
) -> Result<(u64, u64)> {
    let mut ids = ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    let found: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM virtual_api_keys WHERE account_id=$1 AND revoked_at IS NULL AND id=ANY($2)",
    )
    .bind(account_id)
    .bind(&ids)
    .fetch_one(&mut *conn)
    .await?;
    if found != ids.len() as i64 {
        return Err(AppError::BadRequest("部分 Key 不存在或已撤销".into()));
    }
    let left = sqlx::query(
        "UPDATE virtual_api_keys SET group_id=NULL WHERE group_id=$1 AND revoked_at IS NULL AND NOT (id=ANY($2))",
    )
    .bind(group_id)
    .bind(&ids)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    let joined = sqlx::query(
        "UPDATE virtual_api_keys SET group_id=$1 WHERE account_id=$2 AND id=ANY($3) AND group_id IS DISTINCT FROM $1",
    )
    .bind(group_id)
    .bind(account_id)
    .bind(&ids)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok((joined, left))
}

async fn replace_channels(conn: &mut PgConnection, group_id: Uuid, ids: &[Uuid]) -> Result<()> {
    sqlx::query("DELETE FROM key_group_channels WHERE group_id=$1")
        .bind(group_id)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "INSERT INTO key_group_channels(group_id,channel_id) SELECT $1, unnest($2::uuid[])",
    )
    .bind(group_id)
    .bind(ids)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<KeyGroupView>>> {
    let user = auth::current_user(&state, &headers).await?;
    let rows = sqlx::query_as::<_, KeyGroupView>(
        "SELECT g.id,g.name,g.channel_mode,g.model_mode,g.allowed_models,
           ARRAY(SELECT gc.channel_id FROM key_group_channels gc WHERE gc.group_id=g.id) AS channel_ids,
           (SELECT COUNT(*) FROM virtual_api_keys v WHERE v.group_id=g.id AND v.revoked_at IS NULL) AS key_count
         FROM key_groups g WHERE g.account_id=$1 ORDER BY lower(g.name)",
    )
    .bind(user.account_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<KeyGroupInput>,
) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    let scope = validate(&state, user.account_id, input).await?;
    let mut tx = state.db.begin().await?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO key_groups(account_id,name,channel_mode,model_mode,allowed_models) VALUES ($1,$2,$3,$4,$5) RETURNING id",
    )
    .bind(user.account_id)
    .bind(&scope.name)
    .bind(&scope.channel_mode)
    .bind(&scope.model_mode)
    .bind(json!(scope.allowed_models))
    .fetch_one(&mut *tx)
    .await
    .map_err(name_taken)?;
    replace_channels(&mut tx, id, &scope.channel_ids).await?;
    let (joined, _) = match &scope.key_ids {
        Some(ids) => set_members(&mut tx, user.account_id, id, ids).await?,
        None => (0, 0),
    };
    copy_scope(&mut tx, id, None).await?;
    tx.commit().await?;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "key_group.created",
        "key_group",
        id,
        json!({"joined_keys": joined}),
    )
    .await;
    Ok(Json(json!({"id": id})))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<KeyGroupInput>,
) -> Result<StatusCode> {
    let user = auth::current_user(&state, &headers).await?;
    let scope = validate(&state, user.account_id, input).await?;
    let mut tx = state.db.begin().await?;
    let result = sqlx::query(
        "UPDATE key_groups SET name=$1,channel_mode=$2,model_mode=$3,allowed_models=$4 WHERE id=$5 AND account_id=$6",
    )
    .bind(&scope.name)
    .bind(&scope.channel_mode)
    .bind(&scope.model_mode)
    .bind(json!(scope.allowed_models))
    .bind(id)
    .bind(user.account_id)
    .execute(&mut *tx)
    .await
    .map_err(name_taken)?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    replace_channels(&mut tx, id, &scope.channel_ids).await?;
    let (joined, left) = match &scope.key_ids {
        Some(ids) => set_members(&mut tx, user.account_id, id, ids).await?,
        None => (0, 0),
    };
    copy_scope(&mut tx, id, None).await?;
    tx.commit().await?;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "key_group.updated",
        "key_group",
        id,
        json!({"joined_keys": joined, "left_keys": left}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// Keys leave the group with the scope they already carry.
pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let user = auth::current_user(&state, &headers).await?;
    let mut tx = state.db.begin().await?;
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM key_groups WHERE id=$1 AND account_id=$2 FOR UPDATE",
    )
    .bind(id)
    .bind(user.account_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    let detached = sqlx::query("UPDATE virtual_api_keys SET group_id=NULL WHERE group_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    sqlx::query("DELETE FROM key_groups WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "key_group.deleted",
        "key_group",
        id,
        json!({"detached_keys": detached}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}
