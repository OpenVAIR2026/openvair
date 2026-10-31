//! Self-service for people who were handed only a VAIR key. The key itself is
//! the credential, sent in a header exactly as the gateway reads it, and it can
//! read its own status, limits, models and usage, nothing else. Like a VAIR key
//! channel's recipient, the holder never sees the owner's upstream channels.
use crate::{
    SOURCE_KEY_VIEW, auth,
    error::{AppError, Result},
    gateway,
    state::AppState,
    usage::{
        BucketModelRow, BucketPlan, ModelBreakdown, PERFORMANCE_COLUMNS, UsageRange, all_time_plan,
        bucket_plan, build_trend,
    },
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header::CACHE_CONTROL},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::FromRow;
use uuid::Uuid;

#[cfg(test)]
#[path = "key_holder_db_tests.rs"]
mod db_tests;

/// The web page where a key holder pastes the key; gateway errors link here.
pub const PAGE_PATH: &str = "/my-key";
const FAILURE_WINDOW_SECONDS: i64 = 900;
const FAILURE_LIMIT_PER_IP: i64 = 30;

struct HolderKey {
    id: Uuid,
    account_id: Uuid,
}

const FAILURE_SCOPE: &str = "key-holder";

fn failure_key(headers: &HeaderMap, scope: &str) -> Option<String> {
    auth::client_ip(headers).map(|ip| format!("meta-router:{scope}:ip:{}", auth::hash_token(&ip)))
}

/// Keys are long random secrets, so guessing is already hopeless; this budget
/// only keeps one address from hammering the lookup. A Redis outage fails open,
/// like the sign-in limiter.
pub(crate) async fn failure_count(state: &AppState, headers: &HeaderMap, scope: &str) -> i64 {
    let Some(key) = failure_key(headers, scope) else {
        return 0;
    };
    let Ok(mut redis) = state.redis.get_multiplexed_async_connection().await else {
        return 0;
    };
    redis::cmd("GET")
        .arg(&key)
        .query_async::<Option<i64>>(&mut redis)
        .await
        .ok()
        .flatten()
        .unwrap_or(0)
}

pub(crate) async fn record_failure(state: &AppState, headers: &HeaderMap, scope: &str) {
    let Some(key) = failure_key(headers, scope) else {
        return;
    };
    let Ok(mut redis) = state.redis.get_multiplexed_async_connection().await else {
        return;
    };
    let _: std::result::Result<i64, _> = redis::Script::new(
        "local n = redis.call('INCR', KEYS[1]); if n == 1 then redis.call('EXPIRE', KEYS[1], ARGV[1]) end; return n",
    )
    .key(&key)
    .arg(FAILURE_WINDOW_SECONDS)
    .invoke_async(&mut redis)
    .await;
}

/// A paused key still opens its page, so the holder can see why it stopped.
/// Unknown and revoked keys get the same answer as at the gateway.
async fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<HolderKey> {
    if failure_count(state, headers, FAILURE_SCOPE).await >= FAILURE_LIMIT_PER_IP {
        return Err(AppError::TooManyAttempts("尝试次数过多，请稍后再试".into()));
    }
    let key = match gateway::client_token(headers).filter(|token| token.len() <= 512) {
        Some(token) => sqlx::query_as::<_, (Uuid, Uuid)>(
            "SELECT id,account_id FROM virtual_api_keys WHERE key_hash=$1 AND revoked_at IS NULL",
        )
        .bind(auth::hash_token(token))
        .fetch_optional(&state.db)
        .await?,
        None => None,
    };
    match key {
        Some((id, account_id)) => Ok(HolderKey { id, account_id }),
        None => {
            record_failure(state, headers, FAILURE_SCOPE).await;
            Err(AppError::Coded(
                StatusCode::UNAUTHORIZED,
                "invalid_api_key",
                format!("{} Key 无效或已撤销", crate::edition::PRODUCT),
            ))
        }
    }
}

/// Responses describe one key's activity, so shared caches must not keep them.
fn private(body: Value) -> Response {
    ([(CACHE_CONTROL, "no-store")], Json(body)).into_response()
}

/// GET /api/key/summary — status, limits and today's usage, plus the models
/// the key can request now, by protocol.
pub async fn summary(State(state): State<AppState>, headers: HeaderMap) -> Result<Response> {
    let key = authenticate(&state, &headers).await?;
    let (mut view, created_at, last_used_at): (Value, DateTime<Utc>, Option<DateTime<Utc>>) =
        sqlx::query_as(&format!(
            "SELECT ({SOURCE_KEY_VIEW} WHERE v.id=k.id), k.created_at,
               (SELECT MAX(e.created_at) FROM usage_events e
                WHERE e.account_id=k.account_id AND e.virtual_key_id=k.id)
             FROM virtual_api_keys k WHERE k.id=$1 AND k.account_id=$2"
        ))
        .bind(key.id)
        .bind(key.account_id)
        .fetch_one(&state.db)
        .await?;
    // The holder already has the key, so its id reveals nothing new; VAIR
    // Desktop uses it to tell whether an agent still uses this key.
    view["id"] = json!(key.id);
    view["created_at"] = json!(created_at);
    view["last_used_at"] = json!(last_used_at);
    view["models"] = json!(gateway::key_models_by_protocol(&state, key.id).await?);
    // OpenVAIR Desktop offers to move here when it differs from its own address.
    view["server_url"] = json!(crate::public_url::load(&state.db).await);
    Ok(private(view))
}

#[derive(Deserialize)]
pub struct HolderQuery {
    range: Option<UsageRange>,
    /// Browser offset from UTC (local minus UTC), used to align day buckets.
    utc_offset_minutes: Option<i32>,
    status: Option<String>,
    page: Option<i64>,
    limit: Option<i64>,
}

impl HolderQuery {
    async fn plan(&self, state: &AppState, key: &HolderKey) -> Result<BucketPlan> {
        let range = self.range.unwrap_or(UsageRange::Week);
        let offset = self.utc_offset_minutes.unwrap_or(0);
        let now = Utc::now();
        if range != UsageRange::All {
            return Ok(bucket_plan(range, now, offset));
        }
        let first: Option<DateTime<Utc>> = sqlx::query_scalar(
            "SELECT MIN(created_at) FROM usage_events WHERE account_id=$1 AND virtual_key_id=$2",
        )
        .bind(key.account_id)
        .bind(key.id)
        .fetch_one(&state.db)
        .await?;
        Ok(all_time_plan(first.unwrap_or(now), now, offset))
    }
}

/// Binds $1..$3: the key's account, the key and the window start.
const KEY_FILTER: &str = "e.account_id=$1 AND e.virtual_key_id=$2 AND e.created_at>=$3";

#[derive(Serialize, FromRow)]
struct HolderTotals {
    requests: i64,
    successful: i64,
    input_tokens: i64,
    output_tokens: i64,
    cached_tokens: i64,
    known_cost_usd: f64,
    avg_duration_ms: Option<f64>,
}

/// GET /api/key/usage — totals, a trend and a per-model breakdown for the key.
pub async fn usage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HolderQuery>,
) -> Result<Response> {
    let key = authenticate(&state, &headers).await?;
    let plan = query.plan(&state, &key).await?;
    let totals = sqlx::query_as::<_, HolderTotals>(&format!(
        "SELECT COUNT(*)::bigint AS requests,
                COUNT(*) FILTER (WHERE e.status BETWEEN 200 AND 299)::bigint AS successful,
                COALESCE(SUM(e.input_tokens),0)::bigint AS input_tokens,
                COALESCE(SUM(e.output_tokens),0)::bigint AS output_tokens,
                COALESCE(SUM(e.cached_tokens),0)::bigint AS cached_tokens,
                COALESCE(SUM(e.cost_usd),0)::float8 AS known_cost_usd,
                AVG(e.duration_ms)::double precision AS avg_duration_ms
         FROM usage_events e WHERE {KEY_FILTER}"
    ))
    .bind(key.account_id)
    .bind(key.id)
    .bind(plan.start)
    .fetch_one(&state.db)
    .await?;
    let buckets = sqlx::query_as::<_, BucketModelRow>(&format!(
        "SELECT FLOOR(EXTRACT(EPOCH FROM (e.created_at - $3))::double precision / $4::double precision)::bigint AS bucket,
                SUM(e.cost_usd)::float8 AS cost_usd,
                COUNT(*)::bigint AS requests,
                COALESCE(SUM(e.input_tokens),0)::bigint AS input_tokens,
                COALESCE(SUM(e.output_tokens),0)::bigint AS output_tokens,
                {PERFORMANCE_COLUMNS}
         FROM usage_events e WHERE {KEY_FILTER}
         GROUP BY bucket"
    ))
    .bind(key.account_id)
    .bind(key.id)
    .bind(plan.start)
    .bind(plan.step_seconds)
    .fetch_all(&state.db)
    .await?;
    let models = sqlx::query_as::<_, ModelBreakdown>(&format!(
        "SELECT e.model, COUNT(*)::bigint AS requests,
                SUM(e.cost_usd)::float8 AS estimated_cost_usd,
                COALESCE(SUM(COALESCE(e.input_tokens,0)+COALESCE(e.output_tokens,0)),0)::bigint AS tokens
         FROM usage_events e WHERE {KEY_FILTER}
         GROUP BY e.model ORDER BY requests DESC, e.model LIMIT 50"
    ))
    .bind(key.account_id)
    .bind(key.id)
    .bind(plan.start)
    .fetch_all(&state.db)
    .await?;
    Ok(private(json!({
        "since": plan.start,
        "bucket_seconds": plan.step_seconds,
        "totals": totals,
        "trend": build_trend(&plan, buckets),
        "models": models,
    })))
}

/// One request as its key holder sees it: no channel, upstream model or price
/// snapshot, which belong to the owner's setup.
#[derive(Serialize, FromRow)]
struct HolderEvent {
    id: Uuid,
    created_at: DateTime<Utc>,
    model: String,
    protocol: String,
    status: i32,
    ttfb_ms: Option<i32>,
    first_token_ms: Option<i32>,
    stream: Option<bool>,
    duration_ms: i32,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cached_tokens: Option<i64>,
    cost_usd: Option<f64>,
    pricing_status: String,
    /// 'monitor' marks checks run through a VAIR key channel built on this key.
    source: String,
}

/// GET /api/key/events — the key's requests, newest first, ten per page.
pub async fn events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HolderQuery>,
) -> Result<Response> {
    let key = authenticate(&state, &headers).await?;
    let plan = query.plan(&state, &key).await?;
    let status = match query.status.as_deref() {
        None | Some("") => None,
        Some(status @ ("success" | "error")) => Some(status),
        Some(_) => return Err(AppError::BadRequest("invalid status filter".into())),
    };
    let filter = format!(
        "{KEY_FILTER} AND ($4::text IS NULL OR ($4='success' AND e.status BETWEEN 200 AND 299)
           OR ($4='error' AND e.status>=400))"
    );
    let limit = query.limit.unwrap_or(10).clamp(1, 100);
    let total: i64 = sqlx::query_scalar(&format!(
        "SELECT COUNT(*) FROM usage_events e WHERE {filter}"
    ))
    .bind(key.account_id)
    .bind(key.id)
    .bind(plan.start)
    .bind(status)
    .fetch_one(&state.db)
    .await?;
    let page = query
        .page
        .unwrap_or(1)
        .clamp(1, ((total - 1) / limit + 1).max(1));
    let items = sqlx::query_as::<_, HolderEvent>(&format!(
        "SELECT e.id,e.created_at,e.model,e.protocol,e.status,e.ttfb_ms,e.first_token_ms,e.stream,e.duration_ms,
                e.input_tokens,e.output_tokens,e.cached_tokens,e.cost_usd,e.pricing_status,e.source
         FROM usage_events e WHERE {filter}
         ORDER BY e.created_at DESC, e.id DESC LIMIT $5 OFFSET $6"
    ))
    .bind(key.account_id)
    .bind(key.id)
    .bind(plan.start)
    .bind(status)
    .bind(limit)
    .bind((page - 1) * limit)
    .fetch_all(&state.db)
    .await?;
    Ok(private(
        json!({"items": items, "total": total, "page": page, "page_size": limit}),
    ))
}

/// GET /api/key/events/{id}. Explicitly whitelist timing fields: never send
/// raw diagnostics, channel IDs/names, attempt lists or upstream model names.
pub async fn detail(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response> {
    let key = authenticate(&state, &headers).await?;
    #[derive(FromRow)]
    struct DetailRow {
        #[sqlx(flatten)]
        event: HolderEvent,
        request_diagnostics: Option<sqlx::types::Json<crate::request_diagnostics::Diagnostics>>,
    }
    let row = sqlx::query_as::<_, DetailRow>(
        "SELECT id,created_at,model,protocol,status,ttfb_ms,first_token_ms,stream,duration_ms,
          input_tokens,output_tokens,cached_tokens,cost_usd,pricing_status,source,request_diagnostics
         FROM usage_events WHERE account_id=$1 AND virtual_key_id=$2 AND id=$3")
        .bind(key.account_id).bind(key.id).bind(id).fetch_optional(&state.db).await?
        .ok_or(AppError::NotFound)?;
    let mut view = json!(row.event);
    view["timing"] = row.request_diagnostics.map(|timing| {
        let last = timing.attempts.last();
        json!({
            "queue_ms": timing.queue_ms,
            "upstream_headers_ms": last.and_then(|a| a.headers_ms),
            "headers_to_first_token_ms": last.and_then(|a| a.first_token_ms.zip(a.headers_ms)).map(|(first, headers)| first.saturating_sub(headers)),
            "completion": timing.completion,
        })
    }).unwrap_or(Value::Null);
    Ok(private(view))
}

/// GET /api/key/quota-events — how the key's total limit was raised, changed
/// and reset, with the notes the owner left for the holder.
pub async fn quota_history(State(state): State<AppState>, headers: HeaderMap) -> Result<Response> {
    let key = authenticate(&state, &headers).await?;
    Ok(private(json!(
        crate::key_quota::history(&state.db, key.id).await?
    )))
}
