//! Deep checks: a user-started run of fingerprint and tokenizer probes
//! against one monitored channel, judged against the active baselines.
//! Stage one only records the verdict; it never changes routing.
use super::{
    baseline::{self, ActiveBaseline, MAX_REASONING_TOKENS, RANGE_MAX},
    engine::{self, Candidate, MIN_SAMPLES},
    fingerprint,
};
use crate::{
    audit, auth,
    error::{AppError, Result},
    gateway, monitor, pricing,
    quality_monitors::ensure_owned,
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
};
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use rand::{Rng, seq::SliceRandom};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use std::{
    collections::{BTreeMap, HashMap},
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};
use uuid::Uuid;

pub const TIERS: [usize; 3] = [16, 32, 64];
const SAMPLE_RANGE: std::ops::RangeInclusive<usize> = 8..=256;
const FINGERPRINT_MAX_TOKENS: u32 = 2500;
/// Leaves room for thinking when the baseline uses the model's default reasoning.
const FINGERPRINT_MAX_TOKENS_WITH_REASONING: u32 = 8000;
const TOKEN_MAX_TOKENS: u32 = 16;
/// Models with mandatory reasoning may reject an output cap below their thinking budget.
const TOKEN_MAX_TOKENS_WITH_REASONING: u32 = 1024;
const MAX_TOKEN_PROBES: usize = 4;
const N_MIN: usize = 120;
const N_MAX: usize = 300;
const LEASE_SECS: i32 = 600;
/// Upper bound on one deep check, whatever its size.
const MAX_RUNTIME_SECS: u64 = 45 * 60;
const TIMEOUT_MS: u64 = 120_000;

/// Normalizes model names so "anthropic/Claude_Opus-5.5" matches "claude-opus-5-5".
fn normalize(model: &str) -> String {
    model
        .rsplit('/')
        .next()
        .unwrap_or(model)
        .to_ascii_lowercase()
        .replace(['.', '_', ' '], "-")
}

/// The baseline model a channel's upstream (or public) model most likely claims.
pub fn default_claim<'a>(names: &[&str], models: &'a [String]) -> Option<&'a String> {
    names.iter().find_map(|name| {
        let wanted = normalize(name);
        models.iter().find(|m| normalize(m) == wanted)
    })
}

struct MonitorContext {
    model: String,
    upstream_model: String,
    acknowledged: bool,
    daily_sample_limit: i32,
}

async fn monitor_context(db: &PgPool, account_id: Uuid, id: Uuid) -> Result<MonitorContext> {
    let row = sqlx::query(
        "SELECT q.model,q.channel_id,q.daily_sample_limit,c.authenticity_ack_at IS NOT NULL AS acknowledged,
           COALESCE(c.model_mapping->>q.model,r.upstream_model,q.model) AS upstream_model
         FROM quality_monitors q JOIN channels c ON c.id=q.channel_id
         LEFT JOIN LATERAL (SELECT upstream_model FROM model_routes WHERE channel_id=c.id AND account_id=$2
             AND public_model=q.model AND protocol=c.protocol AND enabled ORDER BY priority,id LIMIT 1) r ON true
         WHERE q.id=$1 AND c.account_id=$2 AND q.kind='authenticity' AND q.deleted_at IS NULL",
    )
    .bind(id)
    .bind(account_id)
    .fetch_optional(db)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(MonitorContext {
        model: row.get("model"),
        upstream_model: row.get("upstream_model"),
        acknowledged: row.get("acknowledged"),
        daily_sample_limit: row.get("daily_sample_limit"),
    })
}

/// Choices for the deep-check dialog: claimable models, their evaluated
/// capability, per-sample cost and whether the risk notice was acknowledged.
pub async fn options(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    let ctx = monitor_context(&state.db, user.account_id, id).await?;
    Ok(Json(build_options(&state.db, &ctx, Some(id)).await?))
}

#[derive(Deserialize)]
pub struct NewOptionsQuery {
    pub channel_id: Uuid,
    pub model: String,
}

/// The same choices for a monitor that is still being added.
pub async fn new_options(
    State(state): State<AppState>,
    Query(query): Query<NewOptionsQuery>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    let model = query.model.trim();
    let row = sqlx::query(
        "SELECT c.authenticity_ack_at IS NOT NULL AS acknowledged,
           COALESCE(c.model_mapping->>$3,r.upstream_model,$3) AS upstream_model
         FROM channels c
         LEFT JOIN LATERAL (SELECT upstream_model FROM model_routes WHERE channel_id=c.id AND account_id=$2
             AND public_model=$3 AND protocol=c.protocol AND enabled ORDER BY priority,id LIMIT 1) r ON true
         WHERE c.id=$1 AND c.account_id=$2",
    )
    .bind(query.channel_id)
    .bind(user.account_id)
    .bind(model)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    let ctx = MonitorContext {
        model: model.to_string(),
        upstream_model: row.get("upstream_model"),
        acknowledged: row.get("acknowledged"),
        daily_sample_limit: 0,
    };
    Ok(Json(build_options(&state.db, &ctx, None).await?))
}

async fn build_options(db: &PgPool, ctx: &MonitorContext, id: Option<Uuid>) -> Result<Value> {
    let baselines = baseline::active_baselines(db).await?;
    let names: Vec<String> = baselines.iter().map(|b| b.model.clone()).collect();
    let claimed = default_claim(&[&ctx.upstream_model, &ctx.model], &names);
    let prices = pricing::load(db).await?;
    let evaluations: HashMap<Uuid, (Value, DateTime<Utc>)> = sqlx::query(
        "SELECT DISTINCT ON (version_id) version_id,results,created_at FROM model_baseline_evaluations
         WHERE version_id=ANY($1) ORDER BY version_id,created_at DESC",
    )
    .bind(baselines.iter().map(|b| b.version_id).collect::<Vec<_>>())
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|r| (r.get("version_id"), (r.get("results"), r.get("created_at"))))
    .collect();
    let models: Vec<Value> = baselines
        .iter()
        .map(|b| {
            // Price the channel's own upstream model first; the claimed model is a fallback.
            let price = pricing::lookup(&prices, &ctx.upstream_model)
                .or_else(|| pricing::lookup(&prices, &b.model));
            let per_sample = price.map(|p| {
                p.estimate(b.stats.avg_input.round() as i64, b.stats.avg_output.round() as i64, 0)
            });
            let evaluation = evaluations.get(&b.version_id);
            json!({"model":b.model,"family":b.family,"reasoning":b.reasoning,"per_sample_cost_usd":per_sample,
                "per_sample_tokens":{"input":b.stats.avg_input,"output":b.stats.avg_output},
                "cross_family":evaluation.map(|e| e.0["cross_family"].clone()),
                "same_family":evaluation.map(|e| e.0["same_family"].clone()),
                "has_siblings":evaluation.map(|e| e.0["pairs"].as_array().is_some_and(|p| p.iter().any(|x| x["same_family"]==true))),
                "evaluated_at":evaluation.map(|e| e.1)})
        })
        .collect();
    Ok(
        json!({"monitor_id":id,"model":ctx.model,"upstream_model":ctx.upstream_model,
        "claimed_default":claimed,"acknowledged":ctx.acknowledged,"tiers":TIERS,
        "min_samples":SAMPLE_RANGE.start(),"max_samples":SAMPLE_RANGE.end(),
        "daily_sample_limit":ctx.daily_sample_limit,"models":models}),
    )
}

fn default_true() -> bool {
    true
}
fn default_concurrency() -> usize {
    2
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartInput {
    pub claimed_model: String,
    pub samples: usize,
    #[serde(default = "default_true")]
    pub early_stop: bool,
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    #[serde(default)]
    pub acknowledge_risk: bool,
}

/// Queues a deep check. Returns the job and whether the risk notice was acknowledged now.
pub async fn enqueue(
    db: &PgPool,
    account_id: Uuid,
    user_id: Uuid,
    id: Uuid,
    input: &StartInput,
) -> Result<(Uuid, bool)> {
    if !SAMPLE_RANGE.contains(&input.samples) || !(1..=4).contains(&input.concurrency) {
        return Err(AppError::BadRequest(format!(
            "样本数需要在 {} 到 {} 之间，并发在 1 到 4 之间",
            SAMPLE_RANGE.start(),
            SAMPLE_RANGE.end()
        )));
    }
    let baseline: Option<(Uuid, i32)> = sqlx::query_as(
        "SELECT id,pool_version FROM model_baseline_versions WHERE model=$1 AND status='active'",
    )
    .bind(&input.claimed_model)
    .fetch_optional(db)
    .await?;
    let (baseline_id, pool_version) = baseline
        .ok_or_else(|| AppError::BadRequest(format!("{} 还没有启用的基准", input.claimed_model)))?;
    let mut tx = db.begin().await?;
    let row = sqlx::query(
        "SELECT q.config_version,q.daily_sample_limit,c.enabled,c.authenticity_ack_at IS NOT NULL AS acknowledged
         FROM quality_monitors q JOIN channels c ON c.id=q.channel_id
         WHERE q.id=$1 AND c.account_id=$2 AND q.kind='authenticity' AND q.deleted_at IS NULL FOR UPDATE OF q,c",
    )
    .bind(id)
    .bind(account_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    if !row.get::<bool, _>("enabled") {
        return Err(AppError::Conflict("渠道已停用，请先启用渠道".into()));
    }
    let newly_acknowledged = !row.get::<bool, _>("acknowledged");
    if newly_acknowledged {
        if !input.acknowledge_risk {
            return Err(AppError::Refused(
                "authenticity_ack_required",
                "首次检测前需要确认风险提示".into(),
            ));
        }
        sqlx::query(
            "UPDATE channels c SET authenticity_ack_at=now(),authenticity_ack_by=$3 FROM quality_monitors q
             WHERE q.id=$1 AND c.id=q.channel_id AND c.account_id=$2",
        )
        .bind(id)
        .bind(account_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    }
    if sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM quality_monitor_jobs WHERE monitor_id=$1 AND status IN ('queued','running'))",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?
    {
        return Err(AppError::Conflict(
            "该监控已有检测在排队或运行，请等待完成".into(),
        ));
    }
    let limit = row.get::<i32, _>("daily_sample_limit");
    if limit > 0 {
        let used: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authenticity_samples WHERE monitor_id=$1
             AND created_at >= date_trunc('day',now() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC'",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        if used + input.samples as i64 > i64::from(limit) {
            return Err(AppError::Conflict(format!(
                "今日检测样本额度剩余 {} 个，将在 UTC 00:00 重置",
                (i64::from(limit) - used).max(0)
            )));
        }
    }
    let job: Uuid = sqlx::query_scalar(
        "INSERT INTO quality_monitor_jobs(monitor_id,config_version,source,claimed_model,options,target_samples)
         VALUES($1,$2,'deep',$3,$4,$5) RETURNING id",
    )
    .bind(id)
    .bind(row.get::<i64, _>("config_version"))
    .bind(&input.claimed_model)
    .bind(json!({"early_stop":input.early_stop,"concurrency":input.concurrency,
        "baseline_version_id":baseline_id,"pool_version":pool_version}))
    .bind(input.samples as i32)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((job, newly_acknowledged))
}

pub async fn start(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<StartInput>,
) -> Result<(StatusCode, Json<Value>)> {
    let user = auth::current_user(&state, &headers).await?;
    let (job, acknowledged) = enqueue(&state.db, user.account_id, user.user_id, id, &input).await?;
    if acknowledged {
        audit(
            &state.db,
            user.account_id,
            user.user_id,
            "quality_monitor.authenticity_acknowledged",
            "quality_monitor",
            id,
            json!({}),
        )
        .await;
    }
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "quality_monitor.deep_check",
        "quality_monitor",
        id,
        json!({"job_id":job,"claimed_model":input.claimed_model,"samples":input.samples,
            "early_stop":input.early_stop,"concurrency":input.concurrency}),
    )
    .await;
    Ok((StatusCode::ACCEPTED, Json(json!({"job_id":job}))))
}

fn job_json(r: &sqlx::postgres::PgRow) -> Value {
    json!({"id":r.get::<Uuid,_>("id"),"status":r.get::<String,_>("status"),"source":r.get::<String,_>("source"),
        "claimed_model":r.get::<Option<String>,_>("claimed_model"),"options":r.get::<Option<Value>,_>("options"),
        "target_samples":r.get::<Option<i32>,_>("target_samples"),"completed_samples":r.get::<i32,_>("completed_samples"),
        "valid_samples":r.get::<i32,_>("valid_samples"),"verdict":r.get::<Option<String>,_>("verdict"),
        "evidence":r.get::<Option<Value>,_>("evidence"),"stopped_early":r.get::<bool,_>("stopped_early"),
        "error_code":r.get::<Option<String>,_>("error_code"),"created_at":r.get::<DateTime<Utc>,_>("created_at"),
        "started_at":r.get::<Option<DateTime<Utc>>,_>("started_at"),
        "finished_at":r.get::<Option<DateTime<Utc>>,_>("finished_at")})
}

pub async fn list(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    ensure_owned(&state.db, user.account_id, id, true).await?;
    let rows = sqlx::query(
        "SELECT * FROM quality_monitor_jobs WHERE monitor_id=$1 AND source IN ('deep','scheduled') ORDER BY created_at DESC LIMIT 20",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!(rows.iter().map(job_json).collect::<Vec<_>>())))
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    page: Option<i64>,
    limit: Option<i64>,
    channel_id: Option<Uuid>,
}

/// Every deep check of the account, newest first, including deleted monitors.
pub async fn history(db: &PgPool, account_id: Uuid, query: &HistoryQuery) -> Result<Value> {
    let limit = query.limit.unwrap_or(10).clamp(1, 50);
    let page = query.page.unwrap_or(1).max(1);
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM quality_monitor_jobs j JOIN quality_monitors q ON q.id=j.monitor_id
         JOIN channels c ON c.id=q.channel_id WHERE c.account_id=$1 AND q.kind='authenticity' AND j.source IN ('deep','scheduled')
           AND ($2::uuid IS NULL OR q.channel_id=$2)",
    )
    .bind(account_id)
    .bind(query.channel_id)
    .fetch_one(db)
    .await?;
    let rows = sqlx::query(
        "SELECT j.*,q.id AS monitor_id,q.model,q.channel_id,c.name AS channel_name,q.deleted_at IS NOT NULL AS monitor_deleted
         FROM quality_monitor_jobs j JOIN quality_monitors q ON q.id=j.monitor_id JOIN channels c ON c.id=q.channel_id
         WHERE c.account_id=$1 AND q.kind='authenticity' AND j.source IN ('deep','scheduled') AND ($2::uuid IS NULL OR q.channel_id=$2)
         ORDER BY j.created_at DESC,j.id DESC LIMIT $3 OFFSET $4",
    )
    .bind(account_id)
    .bind(query.channel_id)
    .bind(limit)
    .bind((page - 1) * limit)
    .fetch_all(db)
    .await?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            let mut item = job_json(r);
            item["monitor_id"] = json!(r.get::<Uuid, _>("monitor_id"));
            item["model"] = json!(r.get::<String, _>("model"));
            item["channel_id"] = json!(r.get::<Uuid, _>("channel_id"));
            item["channel_name"] = json!(r.get::<String, _>("channel_name"));
            item["monitor_deleted"] = json!(r.get::<bool, _>("monitor_deleted"));
            item
        })
        .collect();
    Ok(json!({"items":items,"total":total,"page":page,"page_size":limit}))
}

pub async fn list_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    Ok(Json(history(&state.db, user.account_id, &query).await?))
}

pub async fn get(
    State(state): State<AppState>,
    Path((id, job)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    ensure_owned(&state.db, user.account_id, id, true).await?;
    let row = sqlx::query(
        "SELECT * FROM quality_monitor_jobs WHERE id=$1 AND monitor_id=$2 AND source IN ('deep','scheduled')",
    )
    .bind(job)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(job_json(&row)))
}

// ---------------------------------------------------------------- execution

pub(super) struct Probe {
    pub protocol: String,
    pub base_url: String,
    pub secret: String,
    pub model: String,
}

pub(super) struct Reply {
    pub status: Option<u16>,
    pub text: Option<String>,
    pub usage: Option<gateway::TokenUsage>,
    pub reasoning_tokens: Option<i64>,
    pub ttfb_ms: Option<i32>,
    pub latency_ms: i32,
    pub error: Option<String>,
}

impl Reply {
    fn failed(error: String) -> Self {
        Self {
            status: None,
            text: None,
            usage: None,
            reasoning_tokens: None,
            ttfb_ms: None,
            latency_ms: 0,
            error: Some(error),
        }
    }
}

fn reasoning_tokens(protocol: &str, payload: &Value) -> Option<i64> {
    match protocol {
        "gemini" => payload.pointer("/usageMetadata/thoughtsTokenCount"),
        "anthropic" => None,
        _ => payload.pointer("/usage/completion_tokens_details/reasoning_tokens"),
    }
    .and_then(Value::as_i64)
}

/// Sends one probe. Reasoning is requested off where the protocol allows;
/// when an upstream rejects that parameter it is dropped for the rest of the job.
pub(super) async fn send(
    state: &AppState,
    probe: &Probe,
    prompt: &str,
    max_tokens: u32,
    reasoning_param: &AtomicBool,
) -> Reply {
    let started = Instant::now();
    let mut reply = Reply {
        status: None,
        text: None,
        usage: None,
        reasoning_tokens: None,
        ttfb_ms: None,
        latency_ms: 0,
        error: None,
    };
    for attempt in 0..2 {
        let with_reasoning = reasoning_param.load(Ordering::Relaxed);
        let (path, body) = match probe.protocol.as_str() {
            "anthropic" => (
                "/v1/messages".to_owned(),
                json!({"model":probe.model,"max_tokens":max_tokens,"messages":[{"role":"user","content":prompt}]}),
            ),
            "gemini" => {
                let mut config = json!({"maxOutputTokens":max_tokens});
                if with_reasoning {
                    config["thinkingConfig"] = json!({"thinkingBudget":0});
                }
                (
                    format!("/v1beta/models/{}:generateContent", probe.model),
                    json!({"contents":[{"parts":[{"text":prompt}]}],"generationConfig":config}),
                )
            }
            _ => {
                let mut body = json!({"model":probe.model,"messages":[{"role":"user","content":prompt}],
                    "max_tokens":max_tokens,"stream":false});
                if with_reasoning {
                    body["reasoning_effort"] = json!("none");
                }
                ("/v1/chat/completions".to_owned(), body)
            }
        };
        let result = async {
            let target = gateway::join_base_url(&probe.base_url, &path)?;
            gateway::ensure_safe_target(&target).await?;
            let request = state
                .http
                .post(target)
                .timeout(std::time::Duration::from_millis(TIMEOUT_MS))
                .header("content-type", "application/json")
                .body(serde_json::to_vec(&body).unwrap_or_default());
            let request = match probe.protocol.as_str() {
                "anthropic" => request
                    .header("x-api-key", &probe.secret)
                    .header("anthropic-version", "2023-06-01"),
                "gemini" => request.header("x-goog-api-key", &probe.secret),
                _ => request.header(header::AUTHORIZATION, format!("Bearer {}", probe.secret)),
            };
            let response = request
                .send()
                .await
                .map_err(|_| AppError::BadRequest("probe_transport".into()))?;
            let ttfb = started.elapsed().as_millis().min(i32::MAX as u128) as i32;
            let status = response.status().as_u16();
            let mut stream = response.bytes_stream();
            let mut bytes = Vec::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|_| AppError::BadRequest("probe_body".into()))?;
                if bytes.len() + chunk.len() > 1024 * 1024 {
                    return Err(AppError::BadRequest("probe_body_too_large".into()));
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok((status, ttfb, bytes))
        }
        .await;
        reply.latency_ms = started.elapsed().as_millis().min(i32::MAX as u128) as i32;
        match result {
            Err(error) => {
                reply.error = Some(error.to_string());
                return reply;
            }
            Ok((status, ttfb, bytes)) => {
                reply.status = Some(status);
                reply.ttfb_ms = Some(ttfb);
                if status == 400 && with_reasoning && probe.protocol != "anthropic" && attempt == 0
                {
                    reasoning_param.store(false, Ordering::Relaxed);
                    continue;
                }
                if !(200..300).contains(&status) {
                    reply.error = Some(format!("probe_http_{status}"));
                    return reply;
                }
                reply.usage = gateway::parse_usage(&bytes);
                let Ok(payload) = serde_json::from_slice::<Value>(&bytes) else {
                    reply.error = Some("probe_invalid_json".into());
                    return reply;
                };
                reply.reasoning_tokens = reasoning_tokens(&probe.protocol, &payload);
                reply.text = monitor::answer_text(&probe.protocol, &payload);
                return reply;
            }
        }
    }
    reply
}

struct JobContext {
    job_id: Uuid,
    monitor_id: Uuid,
    account_id: Uuid,
    channel_id: Uuid,
    version: i64,
    model: String,
    upstream: Option<Probe>,
    relayed: bool,
    claimed: String,
    target: usize,
    early_stop: bool,
    concurrency: usize,
    /// A scheduled run judges the rolling window instead of its own samples.
    scheduled: bool,
    window_days: i32,
    interval_seconds: i32,
    baseline_id: Option<Uuid>,
}

async fn load_context(state: &AppState, job_id: Uuid) -> Result<Option<JobContext>> {
    let row = sqlx::query(
        "SELECT j.claimed_model,j.options,j.target_samples,j.source,q.id AS monitor_id,q.channel_id,q.model,q.config_version,
           q.window_days,q.interval_seconds,
           c.account_id,c.protocol,c.base_url,c.kind AS channel_kind,
           COALESCE(c.model_mapping->>q.model,r.upstream_model,q.model) AS upstream_model,ck.secret_ciphertext
         FROM quality_monitor_jobs j JOIN quality_monitors q ON q.id=j.monitor_id JOIN channels c ON c.id=q.channel_id
         LEFT JOIN LATERAL(SELECT upstream_model FROM model_routes WHERE channel_id=c.id AND account_id=c.account_id
             AND public_model=q.model AND protocol=c.protocol AND enabled ORDER BY priority,id LIMIT 1) r ON true
         LEFT JOIN LATERAL(SELECT secret_ciphertext FROM channel_keys WHERE channel_id=c.id AND enabled ORDER BY created_at LIMIT 1) ck ON true
         WHERE j.id=$1 AND j.source IN ('deep','scheduled') AND q.kind='authenticity' AND j.status='running'
           AND j.config_version=q.config_version
           AND q.deleted_at IS NULL AND c.enabled",
    )
    .bind(job_id)
    .fetch_optional(&state.db)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let options: Value = row.get("options");
    Ok(Some(JobContext {
        job_id,
        monitor_id: row.get("monitor_id"),
        account_id: row.get("account_id"),
        channel_id: row.get("channel_id"),
        version: row.get("config_version"),
        model: row.get("model"),
        upstream: row
            .get::<Option<String>, _>("secret_ciphertext")
            .and_then(|value| state.crypto.decrypt(&value).ok())
            .map(|secret| Probe {
                protocol: row.get("protocol"),
                base_url: row.get("base_url"),
                secret,
                model: row.get("upstream_model"),
            }),
        relayed: row.get::<String, _>("channel_kind") == "vair_key",
        claimed: row
            .get::<Option<String>, _>("claimed_model")
            .unwrap_or_default(),
        target: row
            .get::<Option<i32>, _>("target_samples")
            .unwrap_or(0)
            .max(0) as usize,
        early_stop: options["early_stop"].as_bool().unwrap_or(true),
        concurrency: options["concurrency"].as_u64().unwrap_or(2).clamp(1, 4) as usize,
        scheduled: row.get::<String, _>("source") == "scheduled",
        window_days: row.get("window_days"),
        interval_seconds: row.get("interval_seconds"),
        baseline_id: None,
    }))
}

/// Where a run's probes go and where its samples are kept: a monitored
/// channel for account checks, a pasted endpoint for public checks.
pub(super) trait ProbeRun {
    async fn probe(
        &self,
        state: &AppState,
        prompt: &str,
        max_tokens: u32,
        reasoning_param: &AtomicBool,
    ) -> Result<Reply>;
    async fn record(&self, state: &AppState, reply: &Reply, sample: &Recorded) -> Result<()>;
    /// False once the run was cancelled or lost its lease.
    async fn still_active(&self, state: &AppState) -> Result<bool>;
    /// Called after every round; `leaning` judges the samples so far.
    async fn progress(
        &self,
        state: &AppState,
        completed: usize,
        valid: usize,
        leaning: Option<&engine::Judgement>,
    ) -> Result<()>;
}

impl ProbeRun for JobContext {
    /// One probe through the channel; a VAIR key channel goes through its
    /// source key so the request counts toward that key's limits.
    async fn probe(
        &self,
        state: &AppState,
        prompt: &str,
        max_tokens: u32,
        reasoning_param: &AtomicBool,
    ) -> Result<Reply> {
        if self.relayed {
            return Ok(
                match gateway::relay_probe_target(
                    state,
                    self.account_id,
                    self.channel_id,
                    &self.model,
                )
                .await?
                {
                    Ok(target) => {
                        let probe = Probe {
                            protocol: target.protocol.clone(),
                            base_url: target.base_url.clone(),
                            secret: target.secret.clone(),
                            model: target.model.clone(),
                        };
                        let reply = send(state, &probe, prompt, max_tokens, reasoning_param).await;
                        target
                            .record(
                                state,
                                reply.status.unwrap_or(502),
                                (reply.ttfb_ms, reply.latency_ms),
                                reply.usage,
                            )
                            .await;
                        reply
                    }
                    Err(reason) => Reply::failed(reason),
                },
            );
        }
        Ok(match &self.upstream {
            Some(probe) => send(state, probe, prompt, max_tokens, reasoning_param).await,
            None => Reply::failed("quality_probe_missing_credential".into()),
        })
    }

    async fn record(&self, state: &AppState, reply: &Reply, sample: &Recorded) -> Result<()> {
        sqlx::query(
            "INSERT INTO authenticity_samples(job_id,monitor_id,probe_kind,template_id,n,sample_values,input_tokens,
             output_tokens,reasoning_tokens,status_code,error_code,valid,invalid_reason,latency_ms,claimed_model,baseline_version_id)
             VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)",
        )
        .bind(self.job_id)
        .bind(self.monitor_id)
        .bind(sample.probe_kind)
        .bind(sample.template_id)
        .bind(sample.n)
        .bind(sample.values.iter().map(|&v| i32::from(v)).collect::<Vec<_>>())
        .bind(sample.input_tokens.map(|v| v.min(i64::from(i32::MAX)) as i32))
        .bind(reply.usage.map(|u| u.output().min(i64::from(i32::MAX)) as i32))
        .bind(reply.reasoning_tokens.map(|v| v.min(i64::from(i32::MAX)) as i32))
        .bind(reply.status.map(i32::from))
        .bind(reply.error.as_deref())
        .bind(sample.valid)
        .bind(sample.invalid_reason.as_deref())
        .bind(reply.latency_ms)
        .bind(&self.claimed)
        .bind(self.baseline_id)
        .execute(&state.db)
        .await?;
        Ok(())
    }

    /// Still running, unchanged and owned by this process's lease.
    async fn still_active(&self, state: &AppState) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM quality_monitor_jobs j JOIN quality_monitors q ON q.id=j.monitor_id
             JOIN channels c ON c.id=q.channel_id WHERE j.id=$1 AND j.status='running' AND j.lease_until>now()
             AND q.config_version=$2 AND q.deleted_at IS NULL AND c.account_id=$3 AND c.enabled)",
        )
        .bind(self.job_id)
        .bind(self.version)
        .bind(self.account_id)
        .fetch_one(&state.db)
        .await?)
    }

    async fn progress(
        &self,
        state: &AppState,
        completed: usize,
        valid: usize,
        _leaning: Option<&engine::Judgement>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE quality_monitor_jobs SET completed_samples=$2,valid_samples=$3,
             lease_until=now()+make_interval(secs=>$4) WHERE id=$1 AND status='running'",
        )
        .bind(self.job_id)
        .bind(completed as i32)
        .bind(valid as i32)
        .bind(LEASE_SECS)
        .execute(&state.db)
        .await?;
        Ok(())
    }
}

pub(super) struct Recorded {
    pub probe_kind: &'static str,
    pub template_id: Uuid,
    pub n: Option<i32>,
    pub values: Vec<u16>,
    pub input_tokens: Option<i64>,
    pub valid: bool,
    pub invalid_reason: Option<String>,
}

/// With `reasoning_off`, a reply that still reasoned is unusable: its
/// baseline was collected without reasoning.
fn fingerprint_sample(reply: &Reply, template_id: Uuid, n: usize, reasoning_off: bool) -> Recorded {
    let mut sample = Recorded {
        probe_kind: "fingerprint",
        template_id,
        n: Some(n as i32),
        values: Vec::new(),
        input_tokens: reply.usage.map(|u| u.input()),
        valid: false,
        invalid_reason: None,
    };
    let reason = if let Some(error) = &reply.error {
        error.clone()
    } else if reasoning_off && reply.reasoning_tokens.unwrap_or(0) > MAX_REASONING_TOKENS {
        "reasoning".into()
    } else {
        match reply
            .text
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            None => "empty".into(),
            Some(text) => {
                match fingerprint::validate(&fingerprint::parse_ints(text), n, RANGE_MAX) {
                    Ok(values) => {
                        sample.values = values;
                        sample.valid = true;
                        return sample;
                    }
                    Err(reason) => reason.into(),
                }
            }
        }
    };
    sample.invalid_reason = Some(reason);
    sample
}

pub(super) fn candidates(
    baselines: &[ActiveBaseline],
    token_pair: Option<(Uuid, Uuid)>,
) -> Vec<Candidate> {
    baselines
        .iter()
        .filter_map(|b| {
            let counts = b.stats.fingerprint.as_ref()?;
            Some(Candidate {
                model: b.model.clone(),
                family: b.family.clone(),
                log_probs: counts.log_probs(),
                token_diff: token_pair.and_then(|(long, short)| {
                    Some(b.stats.token.get(&long)? - b.stats.token.get(&short)?)
                }),
            })
        })
        .collect()
}

pub(super) struct RunOptions {
    pub target: usize,
    pub early_stop: bool,
    pub concurrency: usize,
}

/// What one run of probes collected, before judging.
pub(super) struct Collected {
    pub candidates: Vec<Candidate>,
    pub samples: Vec<Vec<u16>>,
    pub token_diff: Option<f64>,
    pub token_probes: usize,
    pub token_valid: usize,
    /// Input tokens of the shortest token probe beyond the claimed baseline:
    /// a large surplus suggests an injected system prompt.
    pub token_surplus: Option<i64>,
    pub completed: usize,
    pub invalid: BTreeMap<String, usize>,
    pub stopped_early: bool,
    pub stop_reason: Option<&'static str>,
    pub usage: (i64, i64),
    pub reasoning_param_rejected: bool,
}

/// Sends the token probes, then fingerprint probes until the target, an
/// early verdict or a stop condition. None when the run was cancelled.
pub(super) async fn collect(
    state: &AppState,
    run: &impl ProbeRun,
    claimed: &ActiveBaseline,
    baselines: &[ActiveBaseline],
    window_tokens: &HashMap<Uuid, i64>,
    options: &RunOptions,
) -> Result<Option<Collected>> {
    let templates = baseline::pool_templates(state, claimed.pool_version).await?;
    let fingerprint_templates: Vec<_> = templates
        .iter()
        .filter(|t| t.probe_kind == "fingerprint")
        .collect();
    let mut token_templates: Vec<_> = templates
        .iter()
        .filter(|t| t.probe_kind == "token" && claimed.stats.token.contains_key(&t.id))
        .collect();
    // Keep the extremes of the claimed baseline: their difference is the largest signal.
    token_templates
        .sort_by(|a, b| claimed.stats.token[&a.id].total_cmp(&claimed.stats.token[&b.id]));
    if token_templates.len() > MAX_TOKEN_PROBES {
        let last = token_templates.len() - 1;
        token_templates = vec![token_templates[0], token_templates[last]];
    }
    // Match the baseline: turn reasoning off, or send no reasoning parameter.
    let reasoning_off = claimed.reasoning == "off";
    let reasoning_param = AtomicBool::new(reasoning_off);
    let (max_tokens, token_max_tokens) = if reasoning_off {
        (FINGERPRINT_MAX_TOKENS, TOKEN_MAX_TOKENS)
    } else {
        (
            FINGERPRINT_MAX_TOKENS_WITH_REASONING,
            TOKEN_MAX_TOKENS_WITH_REASONING,
        )
    };
    let started = Instant::now();
    let mut usage = (0i64, 0i64);
    let mut observed_tokens: HashMap<Uuid, i64> = HashMap::new();
    for template in &token_templates {
        if let Some(&count) = window_tokens.get(&template.id) {
            observed_tokens.insert(template.id, count);
            continue;
        }
        let reply = run
            .probe(state, &template.content, token_max_tokens, &reasoning_param)
            .await?;
        let input = reply.usage.map(|u| u.input()).filter(|&v| v > 0);
        if let Some(u) = reply.usage {
            usage = (usage.0 + u.input(), usage.1 + u.output());
        }
        let valid = reply.error.is_none() && input.is_some();
        if let Some(input) = input.filter(|_| valid) {
            observed_tokens.insert(template.id, input);
        }
        run.record(
            state,
            &reply,
            &Recorded {
                probe_kind: "token",
                template_id: template.id,
                n: None,
                values: Vec::new(),
                input_tokens: input,
                valid,
                invalid_reason: (!valid)
                    .then(|| reply.error.clone().unwrap_or_else(|| "no_usage".into())),
            },
        )
        .await?;
    }
    let token_pair = (token_templates.len() >= 2).then(|| {
        (
            token_templates[token_templates.len() - 1].id,
            token_templates[0].id,
        )
    });
    let token_diff = token_pair.and_then(|(long, short)| {
        Some((observed_tokens.get(&long)? - observed_tokens.get(&short)?) as f64)
    });
    let token_surplus = token_templates.first().and_then(|t| {
        Some(observed_tokens.get(&t.id)? - claimed.stats.token[&t.id].round() as i64)
    });
    let candidates = candidates(baselines, token_pair);

    let mut samples: Vec<Vec<u16>> = Vec::new();
    let mut invalid: BTreeMap<String, usize> = BTreeMap::new();
    let mut completed = 0usize;
    let mut stopped_early = false;
    let mut stop_reason = None;
    while completed < options.target {
        if fingerprint_templates.is_empty() {
            stop_reason = Some("提示词池中没有指纹模板");
            break;
        }
        if started.elapsed().as_secs() > MAX_RUNTIME_SECS {
            stop_reason = Some("超过单次检测的最长运行时间");
            break;
        }
        if !run.still_active(state).await? {
            return Ok(None);
        }
        let round = options.concurrency.min(options.target - completed);
        let prompts: Vec<(Uuid, usize, String)> = {
            let mut rng = rand::thread_rng();
            (0..round)
                .map(|_| {
                    let n = rng.gen_range(N_MIN..=N_MAX);
                    let template = fingerprint_templates.choose(&mut rng).unwrap();
                    let prompt = template
                        .content
                        .replace("{n}", &n.to_string())
                        .replace("{r}", &RANGE_MAX.to_string());
                    (template.id, n, prompt)
                })
                .collect()
        };
        let replies = futures_util::future::join_all(
            prompts
                .iter()
                .map(|(_, _, prompt)| run.probe(state, prompt, max_tokens, &reasoning_param)),
        )
        .await;
        for ((template_id, n, _), reply) in prompts.iter().zip(replies) {
            let reply = reply?;
            if let Some(u) = reply.usage {
                usage = (usage.0 + u.input(), usage.1 + u.output());
            }
            let sample = fingerprint_sample(&reply, *template_id, *n, reasoning_off);
            run.record(state, &reply, &sample).await?;
            completed += 1;
            if sample.valid {
                samples.push(sample.values);
            } else if let Some(reason) = sample.invalid_reason {
                *invalid.entry(reason).or_default() += 1;
            }
        }
        let leaning = (!samples.is_empty())
            .then(|| engine::judge(&claimed.model, &candidates, &samples, token_diff));
        run.progress(state, completed, samples.len(), leaning.as_ref())
            .await?;
        if completed >= 12 && samples.is_empty() {
            stop_reason = Some("前 12 个样本全部无效，提前结束");
            break;
        }
        if options.early_stop
            && samples.len() >= MIN_SAMPLES
            && completed < options.target
            && leaning.is_some_and(|j| j.verdict.is_final())
        {
            stopped_early = true;
            break;
        }
    }
    Ok(Some(Collected {
        candidates,
        samples,
        token_diff,
        token_probes: token_templates.len(),
        token_valid: observed_tokens.len(),
        token_surplus,
        completed,
        invalid,
        stopped_early,
        stop_reason,
        usage,
        reasoning_param_rejected: reasoning_off && !reasoning_param.load(Ordering::Relaxed),
    }))
}

pub async fn execute(state: &AppState, job_id: Uuid) -> Result<()> {
    sqlx::query(
        "UPDATE quality_monitor_jobs SET lease_until=now()+make_interval(secs=>$2) WHERE id=$1 AND status='running'",
    )
    .bind(job_id)
    .bind(LEASE_SECS)
    .execute(&state.db)
    .await?;
    let Some(mut ctx) = load_context(state, job_id).await? else {
        return Ok(());
    };
    let baselines = baseline::active_baselines(&state.db).await?;
    let Some(claimed) = baselines.iter().find(|b| b.model == ctx.claimed) else {
        return finish(
            state,
            &ctx,
            None,
            json!({"reasons":["申报模型的基准已停用"]}),
            "unsupported",
            false,
        )
        .await;
    };
    ctx.baseline_id = Some(claimed.version_id);
    let ctx = ctx;
    // Token counts are deterministic: a scheduled run reuses the window's
    // counts and probes only templates the window has not measured yet.
    let window_tokens = if ctx.scheduled {
        window_token_counts(state, &ctx).await?
    } else {
        HashMap::new()
    };
    let options = RunOptions {
        target: ctx.target,
        early_stop: ctx.early_stop,
        concurrency: ctx.concurrency,
    };
    let Some(run) = collect(state, &ctx, claimed, &baselines, &window_tokens, &options).await?
    else {
        return Ok(());
    };
    let run_valid = run.samples.len();
    let judged = if ctx.scheduled {
        window_samples(state, &ctx).await?
    } else {
        run.samples
    };
    let judgement = engine::judge(&ctx.claimed, &run.candidates, &judged, run.token_diff);
    let mut evidence = json!(judgement);
    evidence["samples"] = json!({"completed":run.completed,"valid":run_valid,"invalid":run.invalid,
        "target":ctx.target,"token_probes":run.token_probes,"token_valid":run.token_valid});
    if ctx.scheduled {
        evidence["window"] = json!({"days":ctx.window_days,"samples":judged.len()});
    }
    evidence["usage"] = json!({"input_tokens":run.usage.0,"output_tokens":run.usage.1});
    evidence["baseline"] = json!({"version_id":claimed.version_id,"revision":claimed.revision,
        "pool_version":claimed.pool_version});
    evidence["stop_reason"] = json!(run.stop_reason);
    evidence["reasoning_mode"] = json!(claimed.reasoning);
    evidence["reasoning_param_rejected"] = json!(run.reasoning_param_rejected);
    finish(
        state,
        &ctx,
        Some(run.completed),
        evidence,
        judgement.verdict.as_str(),
        run.stopped_early,
    )
    .await
}

async fn finish(
    state: &AppState,
    ctx: &JobContext,
    completed: Option<usize>,
    evidence: Value,
    verdict: &str,
    stopped_early: bool,
) -> Result<()> {
    let mut tx = state.db.begin().await?;
    let current = sqlx::query(
        "SELECT q.config_version,q.deleted_at IS NULL AS live,c.enabled FROM quality_monitors q
         JOIN channels c ON c.id=q.channel_id WHERE q.id=$1 AND c.account_id=$2 FOR UPDATE OF q",
    )
    .bind(ctx.monitor_id)
    .bind(ctx.account_id)
    .fetch_optional(&mut *tx)
    .await?;
    let job_active: bool = sqlx::query_scalar(
        "SELECT status='running' AND lease_until>now() FROM quality_monitor_jobs WHERE id=$1 FOR UPDATE",
    )
    .bind(ctx.job_id)
    .fetch_one(&mut *tx)
    .await?;
    let applied = job_active
        && current.as_ref().is_some_and(|c| {
            c.get::<i64, _>("config_version") == ctx.version
                && c.get::<bool, _>("live")
                && c.get::<bool, _>("enabled")
        });
    sqlx::query(
        "UPDATE quality_monitor_jobs SET status=$2,verdict=$3,evidence=$4,stopped_early=$5,
         completed_samples=COALESCE($6,completed_samples),error_code=$7,finished_at=now() WHERE id=$1 AND status='running'",
    )
    .bind(ctx.job_id)
    .bind(if applied { "completed" } else { "cancelled" })
    .bind(verdict)
    .bind(&evidence)
    .bind(stopped_early)
    .bind(completed.map(|c| c as i32))
    .bind((!applied).then_some("monitor_changed"))
    .execute(&mut *tx)
    .await?;
    if applied && ctx.scheduled {
        sqlx::query(
            "UPDATE quality_monitors SET window_verdict=$2,window_evidence=$3,window_checked_at=now(),
             window_valid_until=now()+make_interval(secs=>$4) WHERE id=$1",
        )
        .bind(ctx.monitor_id)
        .bind(verdict)
        .bind(json!({"job_id":ctx.job_id,"claimed_model":ctx.claimed,"verdict":verdict,
            "valid_samples":evidence["valid_samples"],"best_model":evidence["best_model"],
            "reasons":evidence["reasons"],"window":evidence["window"]}))
        .bind(ctx.interval_seconds.saturating_mul(2))
        .execute(&mut *tx)
        .await?;
    } else if applied {
        sqlx::query(
            "UPDATE quality_monitors SET identity_verdict=$2,identity_evidence=$3,identity_checked_at=now() WHERE id=$1",
        )
        .bind(ctx.monitor_id)
        .bind(verdict)
        .bind(json!({"job_id":ctx.job_id,"claimed_model":ctx.claimed,"verdict":verdict,
            "valid_samples":evidence["valid_samples"],"best_model":evidence["best_model"]}))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    if applied && ctx.scheduled {
        super::schedule::alert(state, ctx.monitor_id, verdict, &evidence).await?;
    }
    Ok(())
}

/// Valid fingerprint samples of the rolling window: scheduled runs under the
/// current configuration, claim and baseline, newest first.
async fn window_samples(state: &AppState, ctx: &JobContext) -> Result<Vec<Vec<u16>>> {
    let rows: Vec<Vec<i32>> = sqlx::query_scalar(
        "SELECT s.sample_values FROM authenticity_samples s JOIN quality_monitor_jobs j ON j.id=s.job_id
         WHERE s.monitor_id=$1 AND j.source='scheduled' AND j.config_version=$2 AND s.claimed_model=$3
           AND s.baseline_version_id=$4 AND s.valid AND s.probe_kind='fingerprint'
           AND s.created_at > now()-make_interval(days=>$5)
         ORDER BY s.created_at DESC,s.id DESC LIMIT 128",
    )
    .bind(ctx.monitor_id)
    .bind(ctx.version)
    .bind(&ctx.claimed)
    .bind(ctx.baseline_id)
    .bind(ctx.window_days)
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|values| {
            values
                .into_iter()
                .filter_map(|v| u16::try_from(v).ok())
                .collect()
        })
        .collect())
}

/// The window's latest valid input-token count per token template.
async fn window_token_counts(state: &AppState, ctx: &JobContext) -> Result<HashMap<Uuid, i64>> {
    let rows: Vec<(Uuid, i32)> = sqlx::query_as(
        "SELECT DISTINCT ON (s.template_id) s.template_id,s.input_tokens FROM authenticity_samples s
         JOIN quality_monitor_jobs j ON j.id=s.job_id
         WHERE s.monitor_id=$1 AND j.source='scheduled' AND j.config_version=$2 AND s.claimed_model=$3
           AND s.baseline_version_id=$4 AND s.valid AND s.probe_kind='token' AND s.input_tokens IS NOT NULL
           AND s.created_at > now()-make_interval(days=>$5)
         ORDER BY s.template_id,s.created_at DESC",
    )
    .bind(ctx.monitor_id)
    .bind(ctx.version)
    .bind(&ctx.claimed)
    .bind(ctx.baseline_id)
    .bind(ctx.window_days)
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, count)| (id, i64::from(count)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claims_match_across_spelling_and_provider_prefixes() {
        let models = vec!["claude-opus-5-5".to_owned(), "qwen3.5-27b".to_owned()];
        assert_eq!(
            default_claim(&["anthropic/Claude_Opus-5.5"], &models),
            Some(&models[0])
        );
        assert_eq!(default_claim(&["qwen3-5-27b"], &models), Some(&models[1]));
        assert_eq!(default_claim(&["gpt-6"], &models), None);
        // The upstream name wins over the public alias.
        assert_eq!(
            default_claim(&["qwen3.5-27b", "claude-opus-5.5"], &models),
            Some(&models[1])
        );
    }

    #[test]
    fn fingerprint_samples_reject_reasoning_and_errors() {
        let ok = Reply {
            status: Some(200),
            text: Some(
                (1..=150)
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
            ),
            usage: None,
            reasoning_tokens: Some(0),
            ttfb_ms: None,
            latency_ms: 1,
            error: None,
        };
        assert!(fingerprint_sample(&ok, Uuid::nil(), 150, true).valid);
        let thinking = Reply {
            reasoning_tokens: Some(900),
            ..ok
        };
        assert_eq!(
            fingerprint_sample(&thinking, Uuid::nil(), 150, true)
                .invalid_reason
                .as_deref(),
            Some("reasoning")
        );
        // Baselines collected with the model's default reasoning accept it.
        assert!(fingerprint_sample(&thinking, Uuid::nil(), 150, false).valid);
        let failed = Reply {
            status: Some(500),
            text: None,
            usage: None,
            reasoning_tokens: None,
            ttfb_ms: None,
            latency_ms: 1,
            error: Some("probe_http_500".into()),
        };
        assert_eq!(
            fingerprint_sample(&failed, Uuid::nil(), 150, true)
                .invalid_reason
                .as_deref(),
            Some("probe_http_500")
        );
    }
}
