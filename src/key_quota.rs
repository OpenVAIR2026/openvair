//! History of a key's total cost limit. Raising the limit, changing it and
//! resetting the usage it counts each leave one record, so the owner and the
//! people the key was handed to read the same account of how much it was given.
//! Records are append-only; a wrong increase is corrected by a later change.
use crate::{
    audit, auth,
    error::{AppError, Result},
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, PgConnection, PgPool};
use uuid::Uuid;

#[cfg(test)]
#[path = "key_quota_db_tests.rs"]
mod db_tests;

const HISTORY_LIMIT: i64 = 200;
const NOTE_MAX_CHARS: usize = 200;
const MAX_INCREASE_USD: f64 = 1_000_000.0;

#[derive(Debug, Serialize, FromRow)]
pub struct QuotaEvent {
    id: Uuid,
    kind: String,
    amount_usd: Option<f64>,
    limit_before_usd: Option<f64>,
    limit_after_usd: Option<f64>,
    used_before_usd: Option<f64>,
    note: Option<String>,
    created_at: DateTime<Utc>,
}

/// One change to record. `kind` is `increase`, `set` or `reset`.
pub(crate) struct QuotaChange {
    pub kind: &'static str,
    pub amount_usd: Option<f64>,
    pub limit_before_usd: Option<f64>,
    pub limit_after_usd: Option<f64>,
    pub used_before_usd: Option<f64>,
    pub note: Option<String>,
}

impl QuotaChange {
    /// A direct change of the limit, including turning it on or off.
    pub fn set(before: Option<f64>, after: Option<f64>) -> Self {
        Self {
            kind: "set",
            amount_usd: None,
            limit_before_usd: before,
            limit_after_usd: after,
            used_before_usd: None,
            note: None,
        }
    }
}

/// Limits are stored to four decimals, so smaller differences are not changes.
pub(crate) fn same_limit(a: Option<f64>, b: Option<f64>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => (a - b).abs() < 1e-9,
        _ => false,
    }
}

fn round_limit(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

pub(crate) async fn record(
    conn: &mut PgConnection,
    key_id: Uuid,
    account_id: Uuid,
    user_id: Uuid,
    change: QuotaChange,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO key_quota_events(virtual_key_id,account_id,kind,amount_usd,limit_before_usd,
           limit_after_usd,used_before_usd,note,created_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
    )
    .bind(key_id)
    .bind(account_id)
    .bind(change.kind)
    .bind(change.amount_usd)
    .bind(change.limit_before_usd)
    .bind(change.limit_after_usd)
    .bind(change.used_before_usd)
    .bind(change.note)
    .bind(user_id)
    .execute(conn)
    .await?;
    Ok(())
}

/// Newest first. The account is checked against the key, so a record can only
/// be read through the key it belongs to.
pub(crate) async fn history(db: &PgPool, key_id: Uuid) -> Result<Vec<QuotaEvent>> {
    Ok(sqlx::query_as::<_, QuotaEvent>(
        "SELECT e.id,e.kind,e.amount_usd,e.limit_before_usd,e.limit_after_usd,e.used_before_usd,
                e.note,e.created_at
         FROM key_quota_events e JOIN virtual_api_keys v ON v.id=e.virtual_key_id AND v.account_id=e.account_id
         WHERE e.virtual_key_id=$1 ORDER BY e.created_at DESC,e.id DESC LIMIT $2",
    )
    .bind(key_id)
    .bind(HISTORY_LIMIT)
    .fetch_all(db)
    .await?)
}

#[derive(Debug, Deserialize)]
pub struct IncreaseInput {
    amount_usd: f64,
    note: Option<String>,
}

fn normalize_note(note: Option<String>) -> Result<Option<String>> {
    let note = note
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if note
        .as_ref()
        .is_some_and(|value| value.chars().count() > NOTE_MAX_CHARS)
    {
        return Err(AppError::BadRequest(format!(
            "备注不能超过 {NOTE_MAX_CHARS} 个字"
        )));
    }
    Ok(note)
}

/// POST /api/keys/{id}/quota-increases — adds to the saved limit on the server,
/// so two people raising it at once both count.
pub async fn increase(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<IncreaseInput>,
) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    if !input.amount_usd.is_finite()
        || input.amount_usd <= 0.0
        || input.amount_usd > MAX_INCREASE_USD
    {
        return Err(AppError::BadRequest("增加金额必须大于 0".into()));
    }
    let note = normalize_note(input.note)?;
    let mut tx = state.db.begin().await?;
    let before: Option<f64> = sqlx::query_scalar(
        "SELECT cost_limit_total_usd FROM virtual_api_keys
         WHERE id=$1 AND account_id=$2 AND revoked_at IS NULL FOR UPDATE",
    )
    .bind(id)
    .bind(user.account_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    let Some(before) = before else {
        return Err(AppError::BadRequest(
            "这把 Key 没有总费用上限，请先设置上限".into(),
        ));
    };
    let amount = round_limit(input.amount_usd);
    let after = round_limit(before + amount);
    sqlx::query("UPDATE virtual_api_keys SET cost_limit_total_usd=$2 WHERE id=$1")
        .bind(id)
        .bind(after)
        .execute(&mut *tx)
        .await?;
    record(
        &mut tx,
        id,
        user.account_id,
        user.user_id,
        QuotaChange {
            kind: "increase",
            amount_usd: Some(amount),
            limit_before_usd: Some(before),
            limit_after_usd: Some(after),
            used_before_usd: None,
            note,
        },
    )
    .await?;
    tx.commit().await?;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "virtual_key.quota_increased",
        "virtual_key",
        id,
        json!({"amount_usd": amount}),
    )
    .await;
    Ok(Json(json!({"cost_limit_total_usd": after})))
}

/// GET /api/keys/{id}/quota-events — the owner's view, revoked keys included.
pub async fn owner_history(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Vec<QuotaEvent>>> {
    let user = auth::current_user(&state, &headers).await?;
    let owned: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM virtual_api_keys WHERE id=$1 AND account_id=$2)",
    )
    .bind(id)
    .bind(user.account_id)
    .fetch_one(&state.db)
    .await?;
    if !owned {
        return Err(AppError::NotFound);
    }
    Ok(Json(history(&state.db, id).await?))
}

/// GET /api/channels/{id}/quota-events — the history of the key behind one of
/// the account's VAIR key channels. A channel whose key is gone has none.
pub async fn channel_history(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Vec<QuotaEvent>>> {
    let user = auth::current_user(&state, &headers).await?;
    let source: Option<Uuid> = sqlx::query_scalar(
        "SELECT source_key_id FROM channels WHERE id=$1 AND account_id=$2 AND kind='vair_key'",
    )
    .bind(id)
    .bind(user.account_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(match source {
        Some(key_id) => history(&state.db, key_id).await?,
        None => Vec::new(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_compare_at_stored_precision() {
        assert!(same_limit(None, None));
        assert!(same_limit(Some(0.1 + 0.2), Some(0.3)));
        assert!(!same_limit(Some(10.0), None));
        assert!(!same_limit(Some(10.0), Some(10.0001)));
        assert_eq!(round_limit(0.1 + 0.2), 0.3);
    }

    #[test]
    fn notes_are_trimmed_and_bounded() {
        assert_eq!(normalize_note(Some("  ".into())).unwrap(), None);
        assert_eq!(
            normalize_note(Some(" 收款 ¥500 ".into()))
                .unwrap()
                .as_deref(),
            Some("收款 ¥500")
        );
        assert!(normalize_note(Some("额".repeat(NOTE_MAX_CHARS + 1))).is_err());
    }
}
