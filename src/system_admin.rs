//! Super-admin tools: account suspension, registration policy, system health
//! and the analysis of successful requests whose cost stayed unknown.
use crate::{
    AppState, audit, auth, email,
    error::{AppError, Result},
    usage::UNKNOWN_COST,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, PgPool};
use std::{
    sync::{
        LazyLock,
        atomic::{AtomicI64, Ordering},
    },
    time::{Duration, Instant},
};
use uuid::Uuid;

static STARTED_AT: LazyLock<DateTime<Utc>> = LazyLock::new(Utc::now);
/// Unix seconds of each background worker's latest loop, 0 before the first.
pub static AVAILABILITY_WORKER_BEAT: AtomicI64 = AtomicI64::new(0);
pub static QUALITY_WORKER_BEAT: AtomicI64 = AtomicI64::new(0);

pub fn mark_started() {
    LazyLock::force(&STARTED_AT);
}

pub fn beat(worker: &AtomicI64) {
    worker.store(Utc::now().timestamp(), Ordering::Relaxed);
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RegistrationSettings {
    pub enabled: bool,
    /// Lowercase email domains allowed to register; empty allows any domain.
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    /// New users confirm their email with a code before the account is created.
    #[serde(default = "default_true")]
    pub require_email_verification: bool,
}

fn default_true() -> bool {
    true
}

pub async fn load_registration(db: &PgPool) -> RegistrationSettings {
    let row: Option<String> =
        sqlx::query_scalar("SELECT value_ciphertext FROM system_settings WHERE key='registration'")
            .fetch_optional(db)
            .await
            .ok()
            .flatten();
    row.and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or(RegistrationSettings {
            enabled: true,
            allowed_domains: Vec::new(),
            require_email_verification: true,
        })
}

/// Verification applies once SMTP is configured, so a fresh instance can still
/// be bootstrapped. An unreadable SMTP configuration counts as configured and
/// fails sign-up instead of silently skipping the check.
pub async fn email_verification_required(
    state: &AppState,
    settings: &RegistrationSettings,
) -> bool {
    settings.require_email_verification
        && !matches!(email::load_smtp_config_optional(state).await, Ok(None))
}

/// Configured super-admins can always register, so a closed instance can be bootstrapped.
pub async fn ensure_registration_allowed(state: &AppState, email: &str) -> Result<()> {
    if state.super_admin_emails.contains(email) {
        return Ok(());
    }
    let settings = load_registration(&state.db).await;
    if !settings.enabled {
        return Err(AppError::Refused(
            "registration_closed",
            "暂未开放注册，请联系管理员".into(),
        ));
    }
    let domain = email.rsplit('@').next().unwrap_or_default();
    if !settings.allowed_domains.is_empty()
        && !settings.allowed_domains.iter().any(|item| item == domain)
    {
        return Err(AppError::Refused(
            "registration_domain_not_allowed",
            "该邮箱域名暂不支持注册".into(),
        ));
    }
    Ok(())
}

fn normalize_domains(values: Vec<String>) -> Result<Vec<String>> {
    let mut domains = values
        .into_iter()
        .map(|value| value.trim().trim_start_matches('@').to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if domains.len() > 50 {
        return Err(AppError::BadRequest("最多设置 50 个邮箱域名".into()));
    }
    for domain in &domains {
        let valid = domain.len() <= 253
            && domain.contains('.')
            && !domain.starts_with(['.', '-'])
            && !domain.ends_with(['.', '-'])
            && !domain.contains("..")
            && domain
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-'));
        if !valid {
            return Err(AppError::BadRequest(format!("邮箱域名无效：{domain}")));
        }
    }
    domains.sort();
    domains.dedup();
    Ok(domains)
}

/// Public: lets the sign-up page explain a closed registration up front.
pub async fn registration_status(State(state): State<AppState>) -> Json<Value> {
    let settings = load_registration(&state.db).await;
    Json(json!({
        "enabled": settings.enabled,
        "domain_restricted": !settings.allowed_domains.is_empty(),
        "email_verification": email_verification_required(&state, &settings).await,
    }))
}

/// Settings plus whether SMTP is configured, which email verification needs.
async fn registration_view(state: &AppState, settings: RegistrationSettings) -> Json<Value> {
    let smtp_configured = !matches!(email::load_smtp_config_optional(state).await, Ok(None));
    Json(json!({
        "enabled": settings.enabled,
        "allowed_domains": settings.allowed_domains,
        "require_email_verification": settings.require_email_verification,
        "smtp_configured": smtp_configured,
    }))
}

pub async fn get_registration(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    auth::require_super_admin(&state, &headers).await?;
    let settings = load_registration(&state.db).await;
    Ok(registration_view(&state, settings).await)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateRegistrationInput {
    enabled: bool,
    #[serde(default)]
    allowed_domains: Vec<String>,
    #[serde(default = "default_true")]
    require_email_verification: bool,
}

pub async fn update_registration(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<UpdateRegistrationInput>,
) -> Result<Json<Value>> {
    let user = auth::require_super_admin(&state, &headers).await?;
    let settings = RegistrationSettings {
        enabled: input.enabled,
        allowed_domains: normalize_domains(input.allowed_domains)?,
        require_email_verification: input.require_email_verification,
    };
    let value = serde_json::to_string(&settings)
        .map_err(|_| AppError::Internal("registration serialization failed".into()))?;
    sqlx::query(
        "INSERT INTO system_settings(key,value_ciphertext) VALUES ('registration',$1)
         ON CONFLICT (key) DO UPDATE SET value_ciphertext=EXCLUDED.value_ciphertext,updated_at=now()",
    )
    .bind(&value)
    .execute(&state.db)
    .await?;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "platform.registration_updated",
        "system_settings",
        user.user_id,
        json!({
            "enabled": settings.enabled,
            "allowed_domains": &settings.allowed_domains,
            "require_email_verification": settings.require_email_verification,
        }),
    )
    .await;
    Ok(registration_view(&state, settings).await)
}

#[derive(Debug, Serialize, FromRow)]
pub struct AdminAccountView {
    id: Uuid,
    name: String,
    created_at: DateTime<Utc>,
    disabled_at: Option<DateTime<Utc>>,
    disabled_reason: Option<String>,
    user_id: Uuid,
    email: String,
    is_super_admin: bool,
    last_active_at: Option<DateTime<Utc>>,
    channel_count: i64,
    key_count: i64,
    requests_30d: i64,
    #[serde(skip)]
    total: i64,
}

#[derive(Debug, Deserialize)]
pub struct AccountListQuery {
    q: Option<String>,
    /// `active` or `disabled`; anything else lists both.
    status: Option<String>,
}

pub async fn list_accounts(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AccountListQuery>,
) -> Result<Json<Value>> {
    auth::require_super_admin(&state, &headers).await?;
    let search = query.q.unwrap_or_default().trim().to_lowercase();
    if search.len() > 320 {
        return Err(AppError::BadRequest("搜索内容过长".into()));
    }
    let status = query
        .status
        .filter(|value| matches!(value.as_str(), "active" | "disabled"));
    let mut accounts = sqlx::query_as::<_, AdminAccountView>(
        "SELECT a.id,a.name,a.created_at,a.disabled_at,a.disabled_reason,
                u.id AS user_id,u.email,u.is_super_admin,
                (SELECT max(COALESCE(s.last_seen_at,s.created_at)) FROM sessions s WHERE s.user_id=u.id) AS last_active_at,
                (SELECT count(*) FROM channels c WHERE c.account_id=a.id) AS channel_count,
                (SELECT count(*) FROM virtual_api_keys v WHERE v.account_id=a.id AND v.revoked_at IS NULL) AS key_count,
                (SELECT count(*) FROM usage_events e WHERE e.account_id=a.id AND e.created_at>=now()-interval '30 days') AS requests_30d,
                count(*) OVER () AS total
         FROM accounts a JOIN memberships m ON m.account_id=a.id JOIN users u ON u.id=m.user_id
         WHERE ($1='' OR strpos(u.email,$1)>0 OR strpos(lower(a.name),$1)>0)
           AND ($2::text IS NULL OR ($2='disabled')=(a.disabled_at IS NOT NULL))
         ORDER BY a.created_at DESC, a.id
         LIMIT 200",
    )
    .bind(&search)
    .bind(status)
    .fetch_all(&state.db)
    .await?;
    let total = accounts.first().map_or(0, |account| account.total);
    for account in &mut accounts {
        account.is_super_admin |= state.super_admin_emails.contains(&account.email);
    }
    Ok(Json(json!({"accounts":accounts,"total":total})))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountStatusInput {
    disabled: bool,
    reason: Option<String>,
}

pub async fn set_account_status(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<AccountStatusInput>,
) -> Result<Json<Value>> {
    let admin = auth::require_super_admin(&state, &headers).await?;
    let reason = input
        .reason
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty() && input.disabled);
    if reason
        .as_ref()
        .is_some_and(|value| value.chars().count() > 200)
    {
        return Err(AppError::BadRequest("停用原因不能超过 200 个字符".into()));
    }
    if input.disabled && id == admin.account_id {
        return Err(AppError::BadRequest("不能停用自己的帐号".into()));
    }
    let mut tx = state.db.begin().await?;
    let target: Option<(Uuid, String, bool)> = sqlx::query_as(
        "SELECT u.id,u.email,u.is_super_admin FROM accounts a
         JOIN memberships m ON m.account_id=a.id JOIN users u ON u.id=m.user_id
         WHERE a.id=$1 FOR UPDATE OF a",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let (user_id, email, is_super_admin) = target.ok_or(AppError::NotFound)?;
    if input.disabled && (is_super_admin || state.super_admin_emails.contains(&email)) {
        return Err(AppError::BadRequest("不能停用超级管理员帐号".into()));
    }
    // Keep the original suspension time when only the reason changes.
    sqlx::query(
        "UPDATE accounts SET disabled_at=CASE WHEN $2 THEN COALESCE(disabled_at,now()) END,
         disabled_reason=$3 WHERE id=$1",
    )
    .bind(id)
    .bind(input.disabled)
    .bind(&reason)
    .execute(&mut *tx)
    .await?;
    let mut signed_out = 0;
    if input.disabled {
        signed_out = sqlx::query("DELETE FROM sessions WHERE user_id=$1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    }
    tx.commit().await?;
    audit(
        &state.db,
        admin.account_id,
        admin.user_id,
        if input.disabled {
            "platform.account_disabled"
        } else {
            "platform.account_enabled"
        },
        "account",
        id,
        json!({"email":email,"reason":reason,"sessions_revoked":signed_out}),
    )
    .await;
    Ok(Json(json!({"id":id,"disabled":input.disabled})))
}

#[derive(Debug, Serialize)]
struct HealthCheck {
    key: &'static str,
    name: &'static str,
    /// `ok`, `warn` or `error`.
    level: &'static str,
    detail: String,
    latency_ms: Option<u128>,
}

impl HealthCheck {
    fn new(key: &'static str, name: &'static str, level: &'static str, detail: String) -> Self {
        Self {
            key,
            name,
            level,
            detail,
            latency_ms: None,
        }
    }
}

fn worker_check(
    key: &'static str,
    name: &'static str,
    beat: &AtomicI64,
    stale_after: i64,
) -> HealthCheck {
    let last = beat.load(Ordering::Relaxed);
    let now = Utc::now().timestamp();
    if last == 0 {
        let starting = now - STARTED_AT.timestamp() < stale_after;
        return HealthCheck::new(
            key,
            name,
            if starting { "ok" } else { "error" },
            if starting {
                "正在启动".into()
            } else {
                "启动后未运行".into()
            },
        );
    }
    let age = now - last;
    HealthCheck::new(
        key,
        name,
        if age <= stale_after { "ok" } else { "error" },
        format!("{age} 秒前运行"),
    )
}

async fn redis_check(state: &AppState) -> HealthCheck {
    let started = Instant::now();
    let ping = async {
        let mut conn = state.redis.get_multiplexed_async_connection().await?;
        let pong: redis::RedisResult<String> = redis::cmd("PING").query_async(&mut conn).await;
        pong
    };
    let mut check = match tokio::time::timeout(Duration::from_secs(2), ping).await {
        Ok(Ok(_)) => HealthCheck::new("redis", "Redis", "ok", "连接正常".into()),
        Ok(Err(_)) => HealthCheck::new("redis", "Redis", "error", "连接失败".into()),
        Err(_) => HealthCheck::new("redis", "Redis", "error", "响应超时".into()),
    };
    check.latency_ms = Some(started.elapsed().as_millis());
    check
}

pub async fn system_health(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    auth::require_super_admin(&state, &headers).await?;
    let mut checks = Vec::new();

    let started = Instant::now();
    let database_ok = sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.db)
        .await
        .is_ok();
    let mut database = HealthCheck::new(
        "database",
        "数据库",
        if database_ok { "ok" } else { "error" },
        format!(
            "连接池 {} 个连接，空闲 {} 个",
            state.db.size(),
            state.db.num_idle()
        ),
    );
    database.latency_ms = Some(started.elapsed().as_millis());
    checks.push(database);
    checks.push(redis_check(&state).await);
    checks.push(worker_check(
        "availability_worker",
        "可用性巡检",
        &AVAILABILITY_WORKER_BEAT,
        900,
    ));
    checks.push(worker_check(
        "quality_worker",
        "质量检测队列",
        &QUALITY_WORKER_BEAT,
        120,
    ));
    let smtp = email::load_smtp_config_optional(&state)
        .await
        .ok()
        .flatten();
    checks.push(match smtp {
        Some(config) => HealthCheck::new(
            "email",
            "邮件发送",
            "ok",
            format!("{}:{}", config.host, config.port),
        ),
        None => HealthCheck::new(
            "email",
            "邮件发送",
            "warn",
            "未配置，找回密码和更换邮箱不可用".into(),
        ),
    });

    let migration: Option<(i64, String)> = sqlx::query_as(
        "SELECT version,description FROM _sqlx_migrations WHERE success ORDER BY version DESC LIMIT 1",
    )
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let stats = sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64)>(
        "SELECT (SELECT count(*) FROM accounts),
                (SELECT count(*) FROM accounts WHERE disabled_at IS NOT NULL),
                (SELECT count(*) FROM sessions WHERE expires_at>now()),
                (SELECT count(*) FROM usage_events WHERE created_at>=now()-interval '24 hours'),
                (SELECT count(*) FROM usage_events WHERE created_at>=now()-interval '24 hours' AND status>=400),
                (SELECT count(*) FROM quality_monitor_jobs WHERE status='queued')",
    )
    .fetch_one(&state.db)
    .await
    .ok();
    let overall = if checks.iter().any(|check| check.level == "error") {
        "error"
    } else if checks.iter().any(|check| check.level == "warn") {
        "warn"
    } else {
        "ok"
    };
    Ok(Json(json!({
        "status": overall,
        "version": env!("CARGO_PKG_VERSION"),
        "started_at": *STARTED_AT,
        "checked_at": Utc::now(),
        "migration": migration.map(|(version, description)| json!({"version":version,"description":description})),
        "checks": checks,
        "stats": stats.map(|(accounts, disabled, sessions, requests, errors, queued)| json!({
            "accounts": accounts,
            "disabled_accounts": disabled,
            "active_sessions": sessions,
            "requests_24h": requests,
            "errors_24h": errors,
            "quality_jobs_queued": queued,
        })),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_registration_domains() {
        assert_eq!(
            normalize_domains(vec![
                " @Example.COM ".into(),
                "example.com".into(),
                "".into()
            ])
            .unwrap(),
            vec!["example.com"]
        );
        assert!(normalize_domains(vec!["localhost".into()]).is_err());
        assert!(normalize_domains(vec!["bad..com".into()]).is_err());
        assert!(normalize_domains(vec!["a b.com".into()]).is_err());
    }
}

#[derive(Debug, Deserialize)]
pub struct UnknownCostQuery {
    days: Option<i32>,
    completion: Option<String>,
    channel_id: Option<Uuid>,
    key_id: Option<Uuid>,
    page: Option<i64>,
}

const UNKNOWN_COST_PAGE: i64 = 50;

/// GET /api/platform/unknown-cost — successful requests without upstream usage
/// across all accounts, filterable by reason, channel and key. Only super
/// admins see these; users are shown priced spend alone.
pub async fn unknown_cost(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<UnknownCostQuery>,
) -> Result<Json<Value>> {
    auth::require_super_admin(&state, &headers).await?;
    let days = query.days.unwrap_or(7).clamp(1, 90);
    let page = query.page.unwrap_or(0).max(0);
    // Streams still running have no cost yet and are not unknown. Unpriced
    // models are only counted: the 待定价 tab lists them. Filter options come
    // from the whole range so picking one never hides the others.
    let sql = format!(
        "WITH ranged AS (
           SELECT e.id,e.created_at,e.account_id,e.virtual_key_id,e.channel_id,e.model,e.upstream_model,
                  e.cost_usd,e.pricing_status,e.duration_ms,e.first_token_ms,
                  e.input_tokens,e.output_tokens,
                  COALESCE(e.request_diagnostics->>'completion','unrecorded') AS completion,
                  {UNKNOWN_COST} AND e.pricing_status<>'unpriced' AS missing,
                  {UNKNOWN_COST} AND e.pricing_status='unpriced' AS unpriced
           FROM usage_events e
           WHERE e.created_at>=date_trunc('day',now())-make_interval(days=>$1-1)
             AND e.status BETWEEN 200 AND 299
             AND (e.in_flight_until IS NULL OR e.in_flight_until<=now())),
         ok AS (SELECT * FROM ranged
                WHERE ($3::uuid IS NULL OR channel_id=$3) AND ($4::uuid IS NULL OR virtual_key_id=$4)),
         missing AS (SELECT * FROM ok WHERE missing AND ($2::text IS NULL OR completion=$2))
         SELECT jsonb_build_object(
           'days',$1,'page',$5,'page_size',$6,
           'totals',(SELECT jsonb_build_object(
               'successful',(SELECT COUNT(*) FROM ok),
               'missing_usage',(SELECT COUNT(*) FROM missing),
               'unpriced',(SELECT COUNT(*) FROM ok WHERE unpriced),
               'priced_cost_usd',(SELECT COALESCE(SUM(cost_usd),0) FROM ok))),
           'reasons',COALESCE((SELECT jsonb_agg(r ORDER BY r.requests DESC) FROM (
               SELECT completion AS value,COUNT(*) AS requests FROM ranged WHERE missing GROUP BY 1) r),'[]'),
           'channels',COALESCE((SELECT jsonb_agg(c ORDER BY c.requests DESC) FROM (
               SELECT e.channel_id AS value,ch.name,a.name AS account_name,COUNT(*) AS requests
               FROM ranged e LEFT JOIN channels ch ON ch.id=e.channel_id
               LEFT JOIN accounts a ON a.id=ch.account_id
               WHERE e.missing AND e.channel_id IS NOT NULL GROUP BY 1,2,3) c),'[]'),
           'keys',COALESCE((SELECT jsonb_agg(k ORDER BY k.requests DESC) FROM (
               SELECT e.virtual_key_id AS value,v.name,a.name AS account_name,COUNT(*) AS requests
               FROM ranged e LEFT JOIN virtual_api_keys v ON v.id=e.virtual_key_id
               LEFT JOIN accounts a ON a.id=e.account_id
               WHERE e.missing AND e.virtual_key_id IS NOT NULL GROUP BY 1,2,3) k),'[]'),
           'rows',COALESCE((SELECT jsonb_agg(x ORDER BY x.created_at DESC) FROM (
               SELECT e.id,e.created_at,a.name AS account_name,v.name AS key_name,ch.name AS channel_name,
                      COALESCE(e.upstream_model,e.model) AS model,e.completion,e.duration_ms,
                      e.first_token_ms,e.input_tokens,e.output_tokens
               FROM missing e LEFT JOIN accounts a ON a.id=e.account_id
               LEFT JOIN virtual_api_keys v ON v.id=e.virtual_key_id
               LEFT JOIN channels ch ON ch.id=e.channel_id
               ORDER BY e.created_at DESC OFFSET $5*$6 LIMIT $6) x),'[]'))"
    );
    let report: Value = sqlx::query_scalar(&sql)
        .bind(days)
        .bind(query.completion.filter(|value| !value.is_empty()))
        .bind(query.channel_id)
        .bind(query.key_id)
        .bind(page)
        .bind(UNKNOWN_COST_PAGE)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(report))
}
