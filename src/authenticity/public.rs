//! Public model verification: a check of one pasted endpoint without an
//! account. The API key and base URL are encrypted in Redis while the check
//! runs and never reach the database or logs; the report keeps only the host,
//! hidden unless the owner shows it.
use super::{
    baseline::{self, ActiveBaseline},
    deep::{self, Probe, ProbeRun, Recorded, Reply, RunOptions},
    engine::{self, Candidate, Judgement},
    fingerprint,
};
use crate::{
    audit, auth, email, email_change,
    error::{AppError, Result},
    gateway, pricing,
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use chrono::{Duration, Utc};
use rand::{Rng, rngs::OsRng};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::Row;
use std::{
    collections::HashMap,
    sync::{Mutex, atomic::AtomicBool},
    time::Duration as StdDuration,
};
use uuid::Uuid;

const TARGET_SAMPLES: usize = 16;
const LEASE_SECS: i64 = 600;
/// A queued check whose key expired can no longer run.
const SECRET_TTL_SECS: i64 = 900;
const DAY_SECS: i64 = 86_400;
/// Public checks per IP and per email each day.
const DAILY_LIMIT: i64 = 3;
/// Must match the `expires_at` default in migrations/0062_public_checks.sql.
const REPORT_DAYS: i64 = 180;
const UNAVAILABLE: &str = "检测服务暂不可用，请稍后重试";
const INVALID_CODE: &str = "验证码无效或已过期，请重新获取";

// ------------------------------------------------------------------ capacity

const CAPACITY_KEY: &str = "public_verify";

/// How much public checking the site takes on, set by a super admin under
/// 系统管理 and stored in `system_settings`. The worker rereads it every loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capacity {
    /// Checks running at once across the site.
    pub workers: i64,
    /// Requests one check keeps in flight to its upstream.
    pub concurrency: i64,
    /// Queued plus running checks before new ones are turned away.
    pub max_waiting: i64,
}

impl Default for Capacity {
    fn default() -> Self {
        Self {
            workers: 4,
            concurrency: 2,
            max_waiting: 20,
        }
    }
}

impl Capacity {
    fn validate(&self) -> Result<()> {
        if !(1..=32).contains(&self.workers) {
            return Err(AppError::BadRequest("同时检测数应在 1 到 32 之间".into()));
        }
        if !(1..=4).contains(&self.concurrency) {
            return Err(AppError::BadRequest("单次检测并发应在 1 到 4 之间".into()));
        }
        if !(self.workers..=500).contains(&self.max_waiting) {
            return Err(AppError::BadRequest(
                "排队上限应不小于同时检测数，且不超过 500".into(),
            ));
        }
        Ok(())
    }
}

/// Falls back to the defaults when the row is missing or unreadable, so a bad
/// value never stops public checks.
pub async fn load_capacity(db: &sqlx::PgPool) -> Capacity {
    let row: Option<String> =
        sqlx::query_scalar("SELECT value_ciphertext FROM system_settings WHERE key=$1")
            .bind(CAPACITY_KEY)
            .fetch_optional(db)
            .await
            .ok()
            .flatten();
    row.and_then(|json| serde_json::from_str::<Capacity>(&json).ok())
        .filter(|capacity| capacity.validate().is_ok())
        .unwrap_or_default()
}

pub async fn get_capacity(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Capacity>> {
    auth::require_super_admin(&state, &headers).await?;
    Ok(Json(load_capacity(&state.db).await))
}

pub async fn update_capacity(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Capacity>,
) -> Result<Json<Capacity>> {
    let user = auth::require_super_admin(&state, &headers).await?;
    input.validate()?;
    let value = serde_json::to_string(&input)
        .map_err(|_| AppError::Internal("capacity serialization failed".into()))?;
    sqlx::query(
        "INSERT INTO system_settings(key,value_ciphertext) VALUES ($1,$2)
         ON CONFLICT (key) DO UPDATE SET value_ciphertext=EXCLUDED.value_ciphertext,updated_at=now()",
    )
    .bind(CAPACITY_KEY)
    .bind(&value)
    .execute(&state.db)
    .await?;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "platform.public_verify_updated",
        "system_settings",
        user.user_id,
        json!(input),
    )
    .await;
    Ok(Json(input))
}

// ------------------------------------------------------------------- public

/// Baselined models a public check can claim, with a cost estimate per check.
pub async fn models(State(state): State<AppState>) -> Result<Json<Value>> {
    let baselines = baseline::active_baselines(&state.db).await?;
    let prices = pricing::load(&state.db).await.unwrap_or_default();
    let models: Vec<Value> = baselines
        .iter()
        .filter(|b| b.stats.fingerprint.is_some())
        .map(|b| {
            let estimate = pricing::lookup(&prices, &b.model).map(|p| {
                p.estimate(
                    (b.stats.avg_input * TARGET_SAMPLES as f64) as i64,
                    (b.stats.avg_output * TARGET_SAMPLES as f64) as i64,
                    0,
                )
            });
            json!({"model":b.model,"family":b.family,"estimated_cost_usd":estimate})
        })
        .collect();
    let capacity = load_capacity(&state.db).await;
    Ok(Json(
        json!({"models":models,"target_samples":TARGET_SAMPLES,"limits":limits(&capacity)}),
    ))
}

fn limits(capacity: &Capacity) -> Value {
    json!({"per_day":DAILY_LIMIT,"concurrent":capacity.workers,"report_days":REPORT_DAYS})
}

/// `email_change::limit` with a message saying which limit was hit; Redis
/// errors pass through unchanged.
async fn limit(
    state: &AppState,
    key: &str,
    maximum: i64,
    seconds: i64,
    message: &str,
) -> Result<()> {
    email_change::limit(state, key, maximum, seconds)
        .await
        .map_err(|error| match error {
            AppError::RateLimited => AppError::TooManyAttempts(message.into()),
            other => other,
        })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeInput {
    email: String,
}

pub async fn request_code(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CodeInput>,
) -> Result<Json<Value>> {
    let email = auth::normalize_email(&input.email)?;
    let smtp = email::load_smtp_config(&state).await?;
    if let Some(ip) = auth::client_ip(&headers) {
        limit(
            &state,
            &format!("public-check-code-ip:{}", auth::hash_token(&ip)),
            10,
            3600,
            "这个网络请求验证码过于频繁，请稍后再试",
        )
        .await?;
    }
    let recipient = auth::hash_token(&email);
    limit(
        &state,
        &format!("public-check-cooldown:{recipient}"),
        1,
        60,
        "验证码已发送，请 60 秒后再重新获取",
    )
    .await?;
    limit(
        &state,
        &format!("public-check-code:{recipient}"),
        5,
        3600,
        "这个邮箱请求验证码次数过多，请一小时后再试",
    )
    .await?;
    let id = Uuid::new_v4();
    let code = format!("{:06}", OsRng.gen_range(0..1_000_000_u32));
    let code_to_hash = code.clone();
    let code_hash = tokio::task::spawn_blocking(move || auth::hash_password(&code_to_hash))
        .await
        .map_err(|_| AppError::Internal("code hashing failed".into()))??;
    sqlx::query(
        "INSERT INTO public_check_verifications(email,id,code_hash,expires_at) VALUES ($1,$2,$3,$4)
         ON CONFLICT (email) DO UPDATE SET id=EXCLUDED.id,code_hash=EXCLUDED.code_hash,
         attempts=0,expires_at=EXCLUDED.expires_at",
    )
    .bind(&email)
    .bind(id)
    .bind(code_hash)
    .bind(Utc::now() + Duration::minutes(10))
    .execute(&state.db)
    .await?;
    if let Err(error) = email::send_public_check_code(smtp, email.clone(), code).await {
        let _ = sqlx::query("DELETE FROM public_check_verifications WHERE email=$1 AND id=$2")
            .bind(&email)
            .bind(id)
            .execute(&state.db)
            .await;
        return Err(error);
    }
    Ok(Json(
        json!({"request_id":id,"expires_in":600,"retry_after":60}),
    ))
}

/// Checks a code without spending it, so a failed connectivity check can be
/// fixed and resubmitted; `spend_code` removes it once the check is queued.
async fn verify_code(state: &AppState, email: &str, request_id: Uuid, code: String) -> Result<()> {
    let mut tx = state.db.begin().await?;
    let row: Option<(Uuid, String, i32)> = sqlx::query_as(
        "SELECT id,code_hash,attempts FROM public_check_verifications
         WHERE email=$1 AND expires_at>now() FOR UPDATE",
    )
    .bind(email)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((id, code_hash, attempts)) = row.filter(|(id, _, _)| *id == request_id) else {
        return Err(AppError::BadRequest(INVALID_CODE.into()));
    };
    if attempts >= 5 {
        return Err(AppError::BadRequest(
            "验证码错误次数过多，请重新获取".into(),
        ));
    }
    let matches = code.len() == 6
        && code.bytes().all(|byte| byte.is_ascii_digit())
        && tokio::task::spawn_blocking(move || auth::verify_password(&code, &code_hash))
            .await
            .map_err(|_| AppError::Internal("code check failed".into()))?;
    if !matches {
        sqlx::query("UPDATE public_check_verifications SET attempts=attempts+1 WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Err(AppError::BadRequest("验证码不正确，请重新输入".into()));
    }
    tx.commit().await?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartInput {
    protocol: String,
    base_url: String,
    api_key: String,
    model: String,
    upstream_model: Option<String>,
    email: String,
    request_id: Uuid,
    code: String,
    accept_risks: bool,
}

#[derive(Serialize, Deserialize)]
struct Secret {
    base_url: String,
    api_key: String,
}

/// The lowercase host of an HTTPS base URL that may be probed from here.
fn public_host(base_url: &str) -> Result<String> {
    let parsed =
        url::Url::parse(base_url).map_err(|_| AppError::BadRequest("接口地址格式不正确".into()))?;
    if parsed.scheme() != "https" {
        return Err(AppError::BadRequest("接口地址必须以 https:// 开头".into()));
    }
    if parsed.query().is_some() {
        return Err(AppError::BadRequest("接口地址不能带查询参数".into()));
    }
    crate::validate_base_url(base_url)
        .map_err(|_| AppError::BadRequest("接口地址不能是内网、本机或带账号信息的地址".into()))?;
    parsed
        .host_str()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| AppError::BadRequest("接口地址缺少域名".into()))
}

fn precheck_error(reply: &Reply) -> String {
    match (reply.status, reply.error.as_deref()) {
        (Some(401 | 403), _) => {
            "渠道拒绝了这个 API Key，请检查 Key 是否正确、是否有这个模型的权限".into()
        }
        (Some(404), _) => "接口地址或模型名不存在，请检查接口地址和渠道里的模型名".into(),
        (Some(429), _) => "渠道当前限流或额度不足，请稍后再试".into(),
        (Some(status), _) if status >= 500 => {
            format!("渠道返回服务器错误（HTTP {status}），请稍后再试")
        }
        (Some(status), Some(error)) if error.starts_with("probe_http_") => {
            format!("渠道拒绝了请求（HTTP {status}），请检查协议和模型名是否匹配")
        }
        (Some(_), _) => "接口返回的内容无法识别，请确认协议选择正确".into(),
        (None, _) => "无法连接到接口地址，请检查地址是否正确、能否从公网访问".into(),
    }
}

fn secret_key(id: Uuid) -> String {
    format!("meta-router:public-check:secret:{id}")
}

async fn redis(state: &AppState) -> Result<redis::aio::MultiplexedConnection> {
    state
        .redis
        .get_multiplexed_async_connection()
        .await
        .map_err(|_| AppError::ServiceUnavailable(UNAVAILABLE.into()))
}

async fn store_secret(state: &AppState, id: Uuid, secret: &Secret) -> Result<()> {
    let plain = serde_json::to_string(secret)
        .map_err(|_| AppError::Internal("secret encoding failed".into()))?;
    let sealed = state
        .crypto
        .encrypt(&plain)
        .map_err(|_| AppError::Internal("secret encryption failed".into()))?;
    let mut conn = redis(state).await?;
    let _: () = redis::cmd("SET")
        .arg(secret_key(id))
        .arg(sealed)
        .arg("EX")
        .arg(SECRET_TTL_SECS)
        .query_async(&mut conn)
        .await
        .map_err(|_| AppError::ServiceUnavailable(UNAVAILABLE.into()))?;
    Ok(())
}

async fn load_secret(state: &AppState, id: Uuid) -> Result<Option<Secret>> {
    let mut conn = redis(state).await?;
    let sealed: Option<String> = redis::cmd("GET")
        .arg(secret_key(id))
        .query_async(&mut conn)
        .await
        .map_err(|_| AppError::ServiceUnavailable(UNAVAILABLE.into()))?;
    Ok(sealed
        .and_then(|value| state.crypto.decrypt(&value).ok())
        .and_then(|plain| serde_json::from_str(&plain).ok()))
}

async fn drop_secret(state: &AppState, id: Uuid) {
    if let Ok(mut conn) = redis(state).await {
        let _: std::result::Result<(), _> = redis::cmd("DEL")
            .arg(secret_key(id))
            .query_async(&mut conn)
            .await;
    }
}

pub async fn start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<StartInput>,
) -> Result<(StatusCode, Json<Value>)> {
    if !input.accept_risks {
        return Err(AppError::BadRequest("请先阅读并确认检测说明".into()));
    }
    if !matches!(input.protocol.as_str(), "openai" | "anthropic" | "gemini") {
        return Err(AppError::BadRequest("不支持的协议".into()));
    }
    let base_url = input.base_url.trim().trim_end_matches('/').to_owned();
    let host = public_host(&base_url)?;
    let api_key = input.api_key.trim().to_owned();
    if api_key.is_empty()
        || api_key.len() > 512
        || api_key.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(AppError::BadRequest("API Key 格式不正确".into()));
    }
    let upstream_model = input
        .upstream_model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .unwrap_or(&input.model)
        .to_owned();
    if upstream_model.len() > 200 || upstream_model.chars().any(char::is_control) {
        return Err(AppError::BadRequest("渠道模型名格式不正确".into()));
    }
    let baselines = baseline::active_baselines(&state.db).await?;
    let Some(claimed) = baselines
        .iter()
        .find(|b| b.model == input.model && b.stats.fingerprint.is_some())
    else {
        return Err(AppError::BadRequest("暂不支持检测这个模型".into()));
    };
    let email = auth::normalize_email(&input.email)?;
    verify_code(&state, &email, input.request_id, input.code).await?;
    let waiting: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public_checks WHERE status IN ('queued','running')",
    )
    .fetch_one(&state.db)
    .await?;
    if waiting >= load_capacity(&state.db).await.max_waiting {
        return Err(AppError::ServiceUnavailable(
            "排队的检测太多，请几分钟后再试".into(),
        ));
    }

    // Limits come before the connectivity check so a caller over the limit
    // never reaches the upstream. A failed connectivity check still counts.
    let ip_hash = auth::client_ip(&headers).map(|ip| auth::hash_token(&ip));
    let email_hash = auth::hash_token(&email);
    if let Some(ip_hash) = &ip_hash {
        limit(
            &state,
            &format!("public-check-ip:{ip_hash}"),
            DAILY_LIMIT,
            DAY_SECS,
            "这个网络今天的 3 次公开检测已用完，明天再试，或注册后在帐号内检测",
        )
        .await?;
    }
    limit(
        &state,
        &format!("public-check-email:{email_hash}"),
        DAILY_LIMIT,
        DAY_SECS,
        "这个邮箱今天的 3 次公开检测已用完，明天再试，或注册后在帐号内检测",
    )
    .await?;
    limit(
        &state,
        &format!("public-check-host:{}", auth::hash_token(&host)),
        30,
        DAY_SECS,
        "这个渠道今天的公开检测次数已达上限，请明天再试",
    )
    .await?;
    limit(
        &state,
        "public-check-global",
        300,
        DAY_SECS,
        "今天全站的公开检测名额已用完，请明天再试，或注册后在帐号内检测",
    )
    .await?;
    if let Some(ip_hash) = &ip_hash {
        limit(
            &state,
            &format!("public-check-precheck:{ip_hash}"),
            20,
            3600,
            "尝试次数过多，请一小时后再试",
        )
        .await?;
    }

    // A one-line request first: a wrong URL, key or model should fail here,
    // before the check is queued.
    gateway::ensure_safe_target(&base_url)
        .await
        .map_err(|_| AppError::BadRequest("接口地址无法解析，或指向了内网地址".into()))?;
    let probe = Probe {
        protocol: input.protocol.clone(),
        base_url: base_url.clone(),
        secret: api_key.clone(),
        model: upstream_model.clone(),
    };
    let reply = deep::send(
        &state,
        &probe,
        "Reply with OK.",
        64,
        &AtomicBool::new(false),
    )
    .await;
    if reply.error.is_some() {
        return Err(AppError::BadRequest(precheck_error(&reply)));
    }

    let id = Uuid::new_v4();
    let token = auth::random_token("vr");
    let manage_token = auth::random_token("vm");
    store_secret(&state, id, &Secret { base_url, api_key }).await?;
    let inserted = sqlx::query(
        "INSERT INTO public_checks(id,token,manage_token_hash,email_hash,ip_hash,protocol,host,claimed_model,
         upstream_model,baseline_version_id,target_samples) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
    )
    .bind(id)
    .bind(&token)
    .bind(auth::hash_token(&manage_token))
    .bind(&email_hash)
    .bind(&ip_hash)
    .bind(&input.protocol)
    .bind(&host)
    .bind(&claimed.model)
    .bind(&upstream_model)
    .bind(claimed.version_id)
    .bind(TARGET_SAMPLES as i32)
    .execute(&state.db)
    .await;
    if let Err(error) = inserted {
        drop_secret(&state, id).await;
        return Err(error.into());
    }
    sqlx::query("DELETE FROM public_check_verifications WHERE email=$1")
        .bind(&email)
        .execute(&state.db)
        .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"token":token,"manage_token":manage_token})),
    ))
}

fn error_message(code: &str) -> &'static str {
    match code {
        "key_expired" => "检测排队太久，临时保存的 Key 已删除，请重新提交",
        "interrupted" => "检测被中断，请重新提交",
        "unsupported" => "申报模型的基准已停用",
        "no_valid_samples" => "渠道的回答都无法使用，没有得出结论",
        _ => "检测出错，请稍后重试",
    }
}

/// Reads a report. The channel host is shown to everyone when the creator
/// chose to share it, and always to the creator holding the manage token.
pub async fn get(
    State(state): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let row = sqlx::query(
        "SELECT p.status,p.protocol,p.host,p.show_host,p.manage_token_hash,p.claimed_model,p.target_samples,p.completed_samples,
           p.valid_samples,p.verdict,p.progress,p.evidence,p.error_code,p.stopped_early,p.cost_usd,
           p.created_at,p.started_at,p.finished_at,
           CASE WHEN p.status='queued' THEN (SELECT count(*) FROM public_checks q
             WHERE q.status='queued' AND q.created_at<p.created_at) END AS ahead
         FROM public_checks p WHERE p.token=$1 AND p.status<>'deleted' AND p.expires_at>now()",
    )
    .bind(&token)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    let error: Option<String> = row.get("error_code");
    let owner = manage_token(&headers).is_some_and(|manage| {
        auth::hash_token(manage) == row.get::<String, _>("manage_token_hash")
    });
    let show_host: bool = row.get("show_host");
    Ok(Json(json!({
        "token": token,
        "status": row.get::<String, _>("status"),
        "protocol": row.get::<String, _>("protocol"),
        "host": (show_host || owner).then(|| row.get::<String, _>("host")),
        "show_host": show_host,
        "owner": owner,
        "claimed_model": row.get::<String, _>("claimed_model"),
        "target_samples": row.get::<i32, _>("target_samples"),
        "completed_samples": row.get::<i32, _>("completed_samples"),
        "valid_samples": row.get::<i32, _>("valid_samples"),
        "queue_ahead": row.get::<Option<i64>, _>("ahead"),
        "verdict": row.get::<Option<String>, _>("verdict"),
        "progress": row.get::<Option<Value>, _>("progress"),
        "evidence": row.get::<Option<Value>, _>("evidence"),
        "error": error.as_deref().map(error_message),
        "stopped_early": row.get::<bool, _>("stopped_early"),
        "cost_usd": row.get::<Option<f64>, _>("cost_usd"),
        "created_at": row.get::<chrono::DateTime<Utc>, _>("created_at"),
        "started_at": row.get::<Option<chrono::DateTime<Utc>>, _>("started_at"),
        "finished_at": row.get::<Option<chrono::DateTime<Utc>>, _>("finished_at"),
    })))
}

/// Deletes a report with the manage token returned when it was created; a
/// running check stops at its next round.
pub async fn delete(
    State(state): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let manage = manage_token(&headers).ok_or(AppError::Unauthorized)?;
    let id: Option<Uuid> = sqlx::query_scalar(
        "UPDATE public_checks SET status='deleted',verdict=NULL,progress=NULL,evidence=NULL,
         finished_at=COALESCE(finished_at,now()) WHERE token=$1 AND manage_token_hash=$2
         AND status<>'deleted' RETURNING id",
    )
    .bind(&token)
    .bind(auth::hash_token(manage))
    .fetch_optional(&state.db)
    .await?;
    let id = id.ok_or(AppError::NotFound)?;
    sqlx::query("DELETE FROM public_check_samples WHERE check_id=$1")
        .bind(id)
        .execute(&state.db)
        .await?;
    drop_secret(&state, id).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareInput {
    show_host: bool,
}

/// Lets the creator choose whether the shared report shows the channel host.
pub async fn update(
    State(state): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
    Json(input): Json<ShareInput>,
) -> Result<StatusCode> {
    let manage = manage_token(&headers).ok_or(AppError::Unauthorized)?;
    let updated = sqlx::query(
        "UPDATE public_checks SET show_host=$3 WHERE token=$1 AND manage_token_hash=$2
         AND status<>'deleted' AND expires_at>now()",
    )
    .bind(&token)
    .bind(auth::hash_token(manage))
    .bind(input.show_host)
    .execute(&state.db)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

fn manage_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("x-manage-token")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
}

// ---------------------------------------------------------------- execution

pub fn start_worker(state: AppState) {
    tokio::spawn(async move {
        let mut running = tokio::task::JoinSet::new();
        loop {
            while running.try_join_next().is_some() {}
            if let Err(error) = expire(&state).await {
                eprintln!("public check expiry failed: {error}");
            }
            let workers = load_capacity(&state.db).await.workers as usize;
            while running.len() < workers {
                match claim(&state).await {
                    Ok(Some(id)) => {
                        let state = state.clone();
                        running.spawn(async move {
                            if let Err(error) = execute(&state, id).await {
                                eprintln!("public check {id} failed: {error}");
                                fail(&state, id, "internal").await;
                            }
                        });
                    }
                    Ok(None) => break,
                    Err(error) => {
                        eprintln!("public check claim failed: {error}");
                        break;
                    }
                }
            }
            tokio::time::sleep(StdDuration::from_secs(2)).await;
        }
    });
}

async fn claim(state: &AppState) -> Result<Option<Uuid>> {
    Ok(sqlx::query_scalar(
        "UPDATE public_checks SET status='running',started_at=now(),lease_until=now()+make_interval(secs=>$1)
         WHERE id=(SELECT id FROM public_checks WHERE status='queued' ORDER BY created_at
           FOR UPDATE SKIP LOCKED LIMIT 1) RETURNING id",
    )
    .bind(LEASE_SECS as f64)
    .fetch_optional(&state.db)
    .await?)
}

/// Fails checks that lost their worker or waited past their key, and drops
/// old reports and codes.
async fn expire(state: &AppState) -> Result<()> {
    let stale: Vec<Uuid> = sqlx::query_scalar(
        "UPDATE public_checks SET status='failed',
           error_code=CASE WHEN status='running' THEN 'interrupted' ELSE 'key_expired' END,finished_at=now()
         WHERE (status='running' AND lease_until<now())
            OR (status='queued' AND created_at<now()-make_interval(secs=>$1)) RETURNING id",
    )
    .bind(SECRET_TTL_SECS as f64)
    .fetch_all(&state.db)
    .await?;
    for id in stale {
        drop_secret(state, id).await;
    }
    sqlx::query("DELETE FROM public_checks WHERE expires_at<now()")
        .execute(&state.db)
        .await?;
    sqlx::query("DELETE FROM public_check_verifications WHERE expires_at<now()")
        .execute(&state.db)
        .await?;
    Ok(())
}

async fn fail(state: &AppState, id: Uuid, code: &str) {
    drop_secret(state, id).await;
    let _ = sqlx::query(
        "UPDATE public_checks SET status='failed',error_code=$2,progress=NULL,finished_at=now()
         WHERE id=$1 AND status='running'",
    )
    .bind(id)
    .bind(code)
    .execute(&state.db)
    .await;
}

/// One row of the report's sample detail: no prompt, answer or probe kind.
#[derive(Serialize)]
struct SampleRow {
    result: &'static str,
    latency_ms: i32,
    leaning: Option<String>,
}

struct PublicRun {
    id: Uuid,
    claimed: String,
    probe: Probe,
    /// Fingerprint distributions, to name the model each sample leans to.
    candidates: Vec<Candidate>,
    rows: Mutex<Vec<SampleRow>>,
}

fn sample_result(reply: &Reply, sample: &Recorded) -> &'static str {
    match reply.status {
        _ if sample.valid => "ok",
        Some(429) => "rate_limited",
        Some(401 | 403) => "rejected",
        Some(status) if status >= 500 => "server_error",
        Some(status) if !(200..300).contains(&status) => "http_error",
        None if reply.error.is_some() => "network",
        _ => "unusable",
    }
}

impl ProbeRun for PublicRun {
    async fn probe(
        &self,
        state: &AppState,
        prompt: &str,
        max_tokens: u32,
        reasoning_param: &AtomicBool,
    ) -> Result<Reply> {
        Ok(deep::send(state, &self.probe, prompt, max_tokens, reasoning_param).await)
    }

    async fn record(&self, state: &AppState, reply: &Reply, sample: &Recorded) -> Result<()> {
        sqlx::query(
            "INSERT INTO public_check_samples(check_id,probe_kind,template_id,n,sample_values,input_tokens,
             output_tokens,status_code,error_code,valid,invalid_reason,ttfb_ms,latency_ms)
             VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
        )
        .bind(self.id)
        .bind(sample.probe_kind)
        .bind(sample.template_id)
        .bind(sample.n)
        .bind(sample.values.iter().map(|&v| i32::from(v)).collect::<Vec<_>>())
        .bind(sample.input_tokens.map(|v| v.min(i64::from(i32::MAX)) as i32))
        .bind(reply.usage.map(|u| u.output().min(i64::from(i32::MAX)) as i32))
        .bind(reply.status.map(i32::from))
        .bind(reply.error.as_deref())
        .bind(sample.valid)
        .bind(sample.invalid_reason.as_deref())
        .bind(reply.ttfb_ms)
        .bind(reply.latency_ms)
        .execute(&state.db)
        .await?;
        let leaning = (sample.probe_kind == "fingerprint" && sample.valid)
            .then(|| {
                self.candidates
                    .iter()
                    .map(|c| (c, fingerprint::mean_loglik(&sample.values, &c.log_probs)))
                    .max_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(c, _)| c.model.clone())
            })
            .flatten();
        self.rows.lock().unwrap().push(SampleRow {
            result: sample_result(reply, sample),
            latency_ms: reply.latency_ms,
            leaning,
        });
        Ok(())
    }

    async fn still_active(&self, state: &AppState) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM public_checks WHERE id=$1 AND status='running' AND lease_until>now())",
        )
        .bind(self.id)
        .fetch_one(&state.db)
        .await?)
    }

    async fn progress(
        &self,
        state: &AppState,
        completed: usize,
        valid: usize,
        leaning: Option<&Judgement>,
    ) -> Result<()> {
        let progress =
            json!({"shares": leaning.map(|j| shares(j, &self.claimed)).unwrap_or_default()});
        sqlx::query(
            "UPDATE public_checks SET completed_samples=$2,valid_samples=$3,progress=$4,
             lease_until=now()+make_interval(secs=>$5) WHERE id=$1 AND status='running'",
        )
        .bind(self.id)
        .bind(completed as i32)
        .bind(valid as i32)
        .bind(progress)
        .bind(LEASE_SECS as f64)
        .execute(&state.db)
        .await?;
        Ok(())
    }
}

/// Relative likelihood of the closest models given the samples so far, from
/// each comparison's mean score difference to the claim.
fn shares(judgement: &Judgement, claimed: &str) -> Vec<Value> {
    if judgement.comparisons.is_empty() {
        return Vec::new();
    }
    let n = judgement.valid_samples as f64;
    let mut logits: Vec<(&str, f64)> = vec![(claimed, 0.0)];
    logits.extend(
        judgement
            .comparisons
            .iter()
            .map(|c| (c.model.as_str(), -c.mean_diff * n)),
    );
    let max = logits.iter().map(|l| l.1).fold(f64::MIN, f64::max);
    let total: f64 = logits.iter().map(|l| (l.1 - max).exp()).sum();
    logits.sort_by(|a, b| b.1.total_cmp(&a.1));
    logits
        .into_iter()
        .take(3)
        .map(|(model, logit)| {
            let share = (logit - max).exp() / total;
            json!({"model":model,"share":(share * 1000.0).round() / 1000.0,"claimed":model == claimed})
        })
        .collect()
}

async fn execute(state: &AppState, id: Uuid) -> Result<()> {
    let row = sqlx::query(
        "SELECT protocol,claimed_model,upstream_model,target_samples FROM public_checks WHERE id=$1 AND status='running'",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let Some(row) = row else {
        return Ok(());
    };
    let Some(secret) = load_secret(state, id).await? else {
        fail(state, id, "key_expired").await;
        return Ok(());
    };
    let baselines = baseline::active_baselines(&state.db).await?;
    let claimed_model: String = row.get("claimed_model");
    let Some(claimed) = baselines.iter().find(|b| b.model == claimed_model) else {
        fail(state, id, "unsupported").await;
        return Ok(());
    };
    let run = PublicRun {
        id,
        claimed: claimed_model.clone(),
        probe: Probe {
            protocol: row.get("protocol"),
            base_url: secret.base_url,
            secret: secret.api_key,
            model: row.get("upstream_model"),
        },
        candidates: deep::candidates(&baselines, None),
        rows: Mutex::new(Vec::new()),
    };
    let options = RunOptions {
        target: row.get::<i32, _>("target_samples").max(1) as usize,
        early_stop: true,
        concurrency: load_capacity(&state.db).await.concurrency as usize,
    };
    let collected =
        deep::collect(state, &run, claimed, &baselines, &HashMap::new(), &options).await;
    drop_secret(state, id).await;
    let Some(collected) = collected? else {
        return Ok(());
    };
    if collected.samples.is_empty() {
        fail(state, id, "no_valid_samples").await;
        return Ok(());
    }
    let judgement = engine::judge(
        &claimed.model,
        &collected.candidates,
        &collected.samples,
        collected.token_diff,
    );
    let cost = pricing::load(&state.db).await.ok().and_then(|prices| {
        pricing::lookup(&prices, &claimed.model)
            .map(|p| p.estimate(collected.usage.0, collected.usage.1, 0))
    });
    let rows = run.rows.into_inner().unwrap_or_default();
    let evidence = report(claimed, &judgement, &collected, rows);
    sqlx::query(
        "UPDATE public_checks SET status='done',verdict=$2,evidence=$3,progress=NULL,completed_samples=$4,
         valid_samples=$5,stopped_early=$6,input_tokens=$7,output_tokens=$8,cost_usd=$9,finished_at=now()
         WHERE id=$1 AND status='running'",
    )
    .bind(id)
    .bind(judgement.verdict.as_str())
    .bind(evidence)
    .bind(collected.completed as i32)
    .bind(collected.samples.len() as i32)
    .bind(collected.stopped_early)
    .bind(collected.usage.0)
    .bind(collected.usage.1)
    .bind(cost)
    .execute(&state.db)
    .await?;
    Ok(())
}

/// Smallest token surplus worth reporting as a likely injected prompt.
const SURPLUS_THRESHOLD: i64 = 200;

/// The public part of a judgement: closest models, the three evidence rows
/// and per-sample outcomes. Raw statistics and reasons stay server-side.
fn report(
    claimed: &ActiveBaseline,
    judgement: &Judgement,
    collected: &deep::Collected,
    rows: Vec<SampleRow>,
) -> Value {
    let tokenizer = judgement.tokenizer.as_ref();
    json!({
        "claimed_family": claimed.family,
        "closest_model": judgement.best_model,
        "shares": shares(judgement, &claimed.model),
        "vendor": {
            "consistent": tokenizer.and_then(|t| t.consistent),
            "families": tokenizer.map(|t| t.matching_families.clone()).unwrap_or_default(),
        },
        "model": {
            "closest": judgement.best_model,
            "same": judgement.best_model.as_deref() == Some(claimed.model.as_str()),
            "closest_same_family": judgement.comparisons.iter()
                .find(|c| judgement.best_model.as_deref() == Some(c.model.as_str()))
                .map(|c| c.same_family),
        },
        "tokens": {
            "surplus": collected.token_surplus.filter(|&s| s >= SURPLUS_THRESHOLD),
        },
        "samples": rows,
    })
}

#[cfg(test)]
pub(super) async fn test_enqueue(
    state: &AppState,
    id: Uuid,
    token: &str,
    manage_token: &str,
    model: &str,
    base_url: &str,
    api_key: &str,
) {
    store_secret(
        state,
        id,
        &Secret {
            base_url: base_url.into(),
            api_key: api_key.into(),
        },
    )
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO public_checks(id,token,manage_token_hash,email_hash,protocol,host,claimed_model,
         upstream_model,target_samples) VALUES($1,$2,$3,'test','openai','1.1.1.1',$4,$4,$5)",
    )
    .bind(id)
    .bind(token)
    .bind(auth::hash_token(manage_token))
    .bind(model)
    .bind(TARGET_SAMPLES as i32)
    .execute(&state.db)
    .await
    .unwrap();
}

#[cfg(test)]
pub(super) async fn run_next(state: &AppState) {
    let id = claim(state).await.unwrap().unwrap();
    execute(state, id).await.unwrap();
}

#[cfg(test)]
pub(super) async fn secret_exists(state: &AppState, id: Uuid) -> bool {
    load_secret(state, id).await.unwrap().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authenticity::engine::{Comparison, Verdict};

    fn judgement(diffs: &[(&str, f64)], valid: usize) -> Judgement {
        Judgement {
            verdict: Verdict::Suspected,
            claimed: "a".into(),
            claimed_family: Some("x".into()),
            valid_samples: valid,
            best_model: None,
            comparisons: diffs
                .iter()
                .map(|(model, mean_diff)| Comparison {
                    model: (*model).into(),
                    family: "x".into(),
                    same_family: true,
                    mean_diff: *mean_diff,
                    z: 0.0,
                    token_consistent: None,
                })
                .collect(),
            tokenizer: None,
            reasons: Vec::new(),
        }
    }

    #[test]
    fn shares_favour_the_model_samples_lean_to() {
        let out = shares(&judgement(&[("b", -0.2), ("c", 0.5)], 10), "a");
        assert_eq!(out[0]["model"], "b");
        assert_eq!(out[1]["model"], "a");
        assert_eq!(out[1]["claimed"], true);
        let total: f64 = out.iter().map(|s| s["share"].as_f64().unwrap()).sum();
        assert!((total - 1.0).abs() < 0.01);
        assert!(shares(&judgement(&[], 10), "a").is_empty());
    }

    #[test]
    fn models_lists_the_public_limits() {
        assert_eq!(
            limits(&Capacity::default()),
            json!({"per_day":3,"concurrent":4,"report_days":180})
        );
    }

    #[test]
    fn capacity_rejects_out_of_range_values() {
        assert!(Capacity::default().validate().is_ok());
        let with = |workers, concurrency, max_waiting| Capacity {
            workers,
            concurrency,
            max_waiting,
        };
        assert!(with(32, 4, 500).validate().is_ok());
        assert!(with(0, 2, 20).validate().is_err());
        assert!(with(33, 2, 40).validate().is_err());
        assert!(with(4, 0, 20).validate().is_err());
        assert!(with(4, 5, 20).validate().is_err());
        assert!(with(8, 2, 7).validate().is_err());
        assert!(with(4, 2, 501).validate().is_err());
    }

    #[test]
    fn public_hosts_must_be_public_https() {
        assert_eq!(
            public_host("https://API.Example.com/v1").unwrap(),
            "api.example.com"
        );
        assert!(public_host("http://api.example.com/v1").is_err());
        assert!(public_host("https://127.0.0.1/v1").is_err());
        assert!(public_host("https://localhost/v1").is_err());
        assert!(public_host("https://user:pw@api.example.com").is_err());
        assert!(public_host("https://api.example.com/v1?key=1").is_err());
    }
}
