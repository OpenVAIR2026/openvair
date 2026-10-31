//! Model-scoped quality monitoring configuration and account-scoped HTTP APIs.
use crate::{
    auth,
    error::{AppError, Result},
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, PgConnection, PgPool, Row};
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub schedule_enabled: bool,
    pub interval_seconds: i32,
    pub timeout_ms: i32,
    pub round_timeout_ms: i32,
    pub methods: Vec<String>,
    pub min_score: f64,
    pub failure_threshold: i32,
    pub daily_run_limit: i32,
    pub route_policy: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            schedule_enabled: false,
            interval_seconds: 3600,
            timeout_ms: 60000,
            round_timeout_ms: 240000,
            methods: crate::service_checks::METHODS.map(str::to_owned).to_vec(),
            min_score: 75.0,
            failure_threshold: 2,
            daily_run_limit: 0,
            route_policy: "observe".into(),
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        if !(900..=86400).contains(&self.interval_seconds)
            || !(10000..=180000).contains(&self.timeout_ms)
            || !(self.timeout_ms..=720000).contains(&self.round_timeout_ms)
            || !(0.0..=100.0).contains(&self.min_score)
            || !(1..=5).contains(&self.failure_threshold)
            || !(0..=10000).contains(&self.daily_run_limit)
        {
            return Err(AppError::BadRequest(
                "检测配置超出范围；整轮时限不能小于单题超时".into(),
            ));
        }
        let unique: std::collections::HashSet<_> = self.methods.iter().collect();
        if self.methods.is_empty()
            || unique.len() != self.methods.len()
            || self
                .methods
                .iter()
                .any(|m| !crate::service_checks::METHODS.contains(&m.as_str()))
        {
            return Err(AppError::BadRequest(
                "请选择至少一种有效且不重复的检测方法".into(),
            ));
        }
        if !["observe", "deprioritize", "exclude"].contains(&self.route_policy.as_str()) {
            return Err(AppError::BadRequest("无效的路由策略".into()));
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateInput {
    pub channel_id: Uuid,
    pub model: String,
    #[serde(default)]
    pub settings: Settings,
    /// Defaults to running only when the channel is enabled; an explicit
    /// `true` for a stopped channel is rejected.
    #[serde(default)]
    pub run_immediately: Option<bool>,
    /// "service" (default) or "authenticity". The two are separate monitors;
    /// only service monitors run service checks and steer routing.
    #[serde(default = "service_kind")]
    pub kind: String,
    /// Authenticity monitors only: settings applied right after creation, in
    /// the same shape as an update (including `acknowledge_risk`).
    #[serde(default)]
    pub authenticity: Option<Value>,
}

fn service_kind() -> String {
    "service".into()
}

#[derive(Serialize, FromRow)]
pub struct MonitorView {
    pub id: Uuid,
    pub kind: String,
    pub channel_id: Uuid,
    pub channel_name: String,
    pub channel_enabled: bool,
    pub protocol: String,
    pub model: String,
    pub upstream_model: String,
    pub schedule_enabled: bool,
    pub interval_seconds: i32,
    pub timeout_ms: i32,
    pub round_timeout_ms: i32,
    pub methods: Value,
    pub min_score: f64,
    pub failure_threshold: i32,
    pub daily_run_limit: i32,
    pub route_policy: String,
    pub status: String,
    pub last_observation: String,
    pub score: Option<f64>,
    pub consecutive_failures: i32,
    pub last_error: Option<String>,
    pub checked_at: Option<DateTime<Utc>>,
    pub valid_until: Option<DateTime<Utc>>,
    pub next_run_at: Option<DateTime<Utc>>,
    pub schedule_error: Option<String>,
    pub config_version: i64,
    pub job_status: Option<String>,
    pub job_error: Option<String>,
    pub identity_verdict: Option<String>,
    pub identity_checked_at: Option<DateTime<Utc>>,
    pub identity_evidence: Option<Value>,
    pub claimed_model: Option<String>,
    pub samples_per_run: i32,
    pub window_days: i32,
    pub daily_sample_limit: i32,
    pub window_verdict: Option<String>,
    pub window_evidence: Option<Value>,
    pub window_checked_at: Option<DateTime<Utc>>,
    pub window_valid_until: Option<DateTime<Utc>>,
    /// Per-check results of the latest applied service-check run, so the list
    /// can show warnings that do not change the monitor's status.
    pub last_tests: Option<Value>,
    pub long_context_max: Option<i32>,
    pub long_context_checked_at: Option<DateTime<Utc>>,
}

pub async fn ensure_owned(
    db: &PgPool,
    account_id: Uuid,
    id: Uuid,
    include_deleted: bool,
) -> Result<()> {
    let found: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM quality_monitors q JOIN channels c ON c.id=q.channel_id
        WHERE q.id=$1 AND c.account_id=$2 AND ($3 OR q.deleted_at IS NULL))",
    )
    .bind(id)
    .bind(account_id)
    .bind(include_deleted)
    .fetch_one(db)
    .await?;
    if !found {
        return Err(AppError::NotFound);
    }
    Ok(())
}

pub async fn load(db: &PgPool, account_id: Uuid) -> Result<Vec<MonitorView>> {
    Ok(sqlx::query_as("SELECT q.id,q.kind,q.channel_id,c.name AS channel_name,c.enabled AS channel_enabled,c.protocol,q.model,
        COALESCE(c.model_mapping->>q.model,r.upstream_model,q.model) AS upstream_model,
        q.schedule_enabled,q.interval_seconds,q.timeout_ms,q.round_timeout_ms,q.methods,q.min_score,
        q.failure_threshold,q.daily_run_limit,q.route_policy,q.status,q.last_observation,q.score,
        q.consecutive_failures,q.last_error,q.checked_at,q.valid_until,q.next_run_at,q.schedule_error,q.config_version,
        j.status AS job_status,j.error_code AS job_error,q.identity_verdict,q.identity_checked_at,q.identity_evidence,q.claimed_model,q.samples_per_run,q.window_days,
        q.daily_sample_limit,q.window_verdict,q.window_evidence,q.window_checked_at,q.window_valid_until,lr.last_tests,
        q.long_context_max,q.long_context_checked_at
        FROM quality_monitors q JOIN channels c ON c.id=q.channel_id
        LEFT JOIN LATERAL (SELECT upstream_model FROM model_routes WHERE channel_id=c.id AND account_id=$1
            AND public_model=q.model AND protocol=c.protocol AND enabled ORDER BY priority,id LIMIT 1) r ON true
        LEFT JOIN LATERAL (SELECT status,error_code FROM quality_monitor_jobs WHERE monitor_id=q.id AND source<>'deep' ORDER BY created_at DESC,id DESC LIMIT 1) j ON true
        LEFT JOIN LATERAL (SELECT details->'tests' AS last_tests FROM channel_monitor_runs WHERE monitor_id=q.id AND kind='quality'
            AND details->>'suite_version' IN ('v3','v4','v5','v6') AND COALESCE((details->>'applied')::boolean,true)
            ORDER BY created_at DESC,id DESC LIMIT 1) lr ON true
        WHERE c.account_id=$1 AND q.deleted_at IS NULL AND ($2 OR q.kind='service') ORDER BY q.created_at DESC,q.id")
        .bind(account_id).bind(crate::edition::CLOUD).fetch_all(db).await?)
}

pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<MonitorView>>> {
    let user = auth::current_user(&state, &headers).await?;
    Ok(Json(load(&state.db, user.account_id).await?))
}

pub async fn insert(
    db: &PgPool,
    account_id: Uuid,
    input: &CreateInput,
) -> Result<(Uuid, Option<Uuid>)> {
    input.settings.validate()?;
    let authenticity = match input.kind.as_str() {
        "service" => false,
        "authenticity" if crate::edition::CLOUD => true,
        "authenticity" => {
            return Err(AppError::BadRequest("当前版本不提供真伪检测".into()));
        }
        _ => {
            return Err(AppError::BadRequest(
                "监控类型必须是 service 或 authenticity".into(),
            ));
        }
    };
    let model = input.model.trim();
    if model.is_empty()
        || model.len() > 256
        || model.chars().any(char::is_control)
        || model.contains('*')
    {
        return Err(AppError::BadRequest(
            "请选择一个具体模型，模型名不能包含通配符".into(),
        ));
    }
    let mut tx = db.begin().await?;
    let channel: Option<(bool, bool)> = sqlx::query_as("SELECT c.model_mode='passthrough' OR c.models ? $3
        OR COALESCE(c.test_model=$3,false) OR c.model_mapping ? $3
        OR EXISTS(SELECT 1 FROM model_routes r WHERE r.channel_id=c.id AND r.account_id=$2 AND r.protocol=c.protocol AND r.public_model=$3 AND r.enabled)
        OR EXISTS(SELECT 1 FROM jsonb_array_elements_text(c.models) m WHERE right(m,1)='*' AND left($3,length(m)-1)=left(m,length(m)-1)),
        c.enabled FROM channels c WHERE c.id=$1 AND c.account_id=$2 FOR SHARE")
        .bind(input.channel_id).bind(account_id).bind(model).fetch_optional(&mut *tx).await?;
    let (supported, channel_enabled) = channel.ok_or(AppError::NotFound)?;
    if !supported {
        return Err(AppError::BadRequest(
            "该模型不在渠道支持的模型范围内".into(),
        ));
    }
    // Authenticity monitors run only on demand and never steer routing yet.
    let defaults = Settings {
        interval_seconds: 21600,
        ..Settings::default()
    };
    let s = if authenticity {
        &defaults
    } else {
        &input.settings
    };
    let id: Option<Uuid> = sqlx::query_scalar("INSERT INTO quality_monitors(channel_id,model,kind,schedule_enabled,interval_seconds,
        timeout_ms,round_timeout_ms,methods,min_score,failure_threshold,daily_run_limit,route_policy,next_run_at)
        VALUES($1,$2,$12,$3,$4,$5,$6,$7,$8,$9,$10,$11,CASE WHEN $3 THEN now() END)
        ON CONFLICT (channel_id,model,kind) WHERE deleted_at IS NULL DO NOTHING RETURNING id")
        .bind(input.channel_id).bind(model).bind(s.schedule_enabled).bind(s.interval_seconds).bind(s.timeout_ms)
        .bind(s.round_timeout_ms).bind(json!(s.methods)).bind(s.min_score).bind(s.failure_threshold)
        .bind(s.daily_run_limit).bind(&s.route_policy).bind(&input.kind).fetch_optional(&mut *tx).await?;
    let id = id.ok_or_else(|| {
        AppError::Conflict(if authenticity {
            "该渠道和模型已有真伪检测监控".into()
        } else {
            "该渠道和模型已有服务体检监控，请编辑现有监控".into()
        })
    })?;
    let job = if !authenticity && input.run_immediately.unwrap_or(channel_enabled) {
        Some(enqueue_tx(&mut tx, account_id, id, "create").await?)
    } else {
        None
    };
    tx.commit().await?;
    Ok((id, job))
}

/// Inserts a monitor; an authenticity monitor also gets its suggested claim
/// and any settings sent with it.
pub async fn create_monitor(
    db: &PgPool,
    account_id: Uuid,
    user_id: Uuid,
    input: &CreateInput,
) -> Result<(Uuid, Option<Uuid>)> {
    let (id, job) = insert(db, account_id, input).await?;
    if input.kind == "authenticity" {
        crate::authenticity::schedule::set_default_claim(db, account_id, id).await?;
        if let Some(settings) = &input.authenticity {
            // A rejected setting leaves nothing behind: the monitor has no jobs or samples yet.
            if let Err(error) =
                crate::authenticity::schedule::patch(db, account_id, user_id, id, settings).await
            {
                sqlx::query("DELETE FROM quality_monitors WHERE id=$1")
                    .bind(id)
                    .execute(db)
                    .await?;
                return Err(error);
            }
        }
    }
    Ok((id, job))
}

pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateInput>,
) -> Result<(StatusCode, Json<Value>)> {
    let user = auth::current_user(&state, &headers).await?;
    let (id, job) = create_monitor(&state.db, user.account_id, user.user_id, &input).await?;
    crate::audit(&state.db,user.account_id,user.user_id,"quality_monitor.created","quality_monitor",id,
        json!({"channel_id":input.channel_id,"model":input.model,"kind":input.kind,"settings":input.settings,"job_id":job})).await;
    Ok((StatusCode::CREATED, Json(json!({"id":id,"job_id":job}))))
}

#[cfg(test)]
pub async fn change_settings(
    db: &PgPool,
    account_id: Uuid,
    id: Uuid,
    settings: &Settings,
) -> Result<()> {
    patch_settings(db, account_id, id, &json!(settings))
        .await
        .map(|_| ())
}

pub async fn patch_settings(
    db: &PgPool,
    account_id: Uuid,
    id: Uuid,
    patch: &Value,
) -> Result<Settings> {
    let patch = patch
        .as_object()
        .ok_or_else(|| AppError::BadRequest("监控配置必须是 JSON 对象".into()))?;
    let mut tx = db.begin().await?;
    let row = sqlx::query(
        "SELECT q.* FROM quality_monitors q JOIN channels c ON c.id=q.channel_id
        WHERE q.id=$1 AND c.account_id=$2 AND q.deleted_at IS NULL FOR UPDATE OF q",
    )
    .bind(id)
    .bind(account_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    if row.get::<String, _>("kind") != "service" {
        return Err(AppError::BadRequest(
            "真伪检测监控没有可编辑的检测设置".into(),
        ));
    }
    let mut merged = json!({
        "schedule_enabled":row.get::<bool,_>("schedule_enabled"),
        "interval_seconds":row.get::<i32,_>("interval_seconds"),
        "timeout_ms":row.get::<i32,_>("timeout_ms"),
        "round_timeout_ms":row.get::<i32,_>("round_timeout_ms"),
        "methods":row.get::<Value,_>("methods"),
        "min_score":row.get::<f64,_>("min_score"),
        "failure_threshold":row.get::<i32,_>("failure_threshold"),
        "daily_run_limit":row.get::<i32,_>("daily_run_limit"),
        "route_policy":row.get::<String,_>("route_policy")
    });
    for (key, value) in patch {
        merged[key] = value.clone();
    }
    let settings: Settings = serde_json::from_value(merged)
        .map_err(|_| AppError::BadRequest("监控配置字段无效；渠道和模型不可修改".into()))?;
    settings.validate()?;
    let changed = row.get::<Value, _>("methods") != json!(settings.methods)
        || row.get::<f64, _>("min_score") != settings.min_score
        || row.get::<i32, _>("failure_threshold") != settings.failure_threshold
        || row.get::<i32, _>("timeout_ms") != settings.timeout_ms
        || row.get::<i32, _>("round_timeout_ms") != settings.round_timeout_ms;
    sqlx::query("UPDATE quality_monitors SET schedule_enabled=$2,interval_seconds=$3,timeout_ms=$4,round_timeout_ms=$5,
        methods=$6,min_score=$7,failure_threshold=$8,daily_run_limit=$9,route_policy=$10,
        config_version=config_version+CASE WHEN $11 THEN 1 ELSE 0 END,
        status=CASE WHEN $11 THEN 'unknown' ELSE status END,last_observation=CASE WHEN $11 THEN 'unknown' ELSE last_observation END,
        score=CASE WHEN $11 THEN NULL ELSE score END,consecutive_failures=CASE WHEN $11 THEN 0 ELSE consecutive_failures END,
        valid_until=CASE WHEN $11 THEN NULL
            ELSE valid_until+make_interval(secs=>2*($3-interval_seconds)) END,checked_at=CASE WHEN $11 THEN NULL ELSE checked_at END,
        last_error=CASE WHEN $11 THEN NULL ELSE last_error END,schedule_error=NULL,
        next_run_at=CASE WHEN NOT $2 THEN NULL WHEN $11 OR NOT schedule_enabled OR interval_seconds<>$3 THEN now() ELSE COALESCE(next_run_at,now()) END,updated_at=now()
        WHERE id=$1")
        .bind(id).bind(settings.schedule_enabled).bind(settings.interval_seconds).bind(settings.timeout_ms)
        .bind(settings.round_timeout_ms).bind(json!(settings.methods)).bind(settings.min_score).bind(settings.failure_threshold)
        .bind(settings.daily_run_limit).bind(&settings.route_policy).bind(changed).execute(&mut *tx).await?;
    sqlx::query("UPDATE quality_monitor_jobs SET status='cancelled',finished_at=now(),error_code='monitor_changed'
        WHERE monitor_id=$1 AND status IN ('queued','running') AND ($2 OR (source='scheduled' AND NOT $3))")
        .bind(id).bind(changed).bind(settings.schedule_enabled).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(settings)
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(patch): Json<Value>,
) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    let kind: Option<String> = sqlx::query_scalar(
        "SELECT q.kind FROM quality_monitors q JOIN channels c ON c.id=q.channel_id
         WHERE q.id=$1 AND c.account_id=$2 AND q.deleted_at IS NULL",
    )
    .bind(id)
    .bind(user.account_id)
    .fetch_optional(&state.db)
    .await?;
    let saved = match kind.as_deref().ok_or(AppError::NotFound)? {
        "authenticity" => json!(
            crate::authenticity::schedule::patch(
                &state.db,
                user.account_id,
                user.user_id,
                id,
                &patch
            )
            .await?
        ),
        _ => json!(patch_settings(&state.db, user.account_id, id, &patch).await?),
    };
    crate::audit(
        &state.db,
        user.account_id,
        user.user_id,
        "quality_monitor.updated",
        "quality_monitor",
        id,
        saved,
    )
    .await;
    Ok(Json(json!({"id":id})))
}

pub async fn remove(db: &PgPool, account_id: Uuid, id: Uuid) -> Result<()> {
    let mut tx = db.begin().await?;
    let n = sqlx::query(
        "UPDATE quality_monitors q SET deleted_at=now(),schedule_enabled=false,next_run_at=NULL,
        valid_until=NULL,config_version=config_version+1,updated_at=now() FROM channels c
        WHERE q.id=$1 AND c.id=q.channel_id AND c.account_id=$2 AND q.deleted_at IS NULL",
    )
    .bind(id)
    .bind(account_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(AppError::NotFound);
    }
    sqlx::query("UPDATE quality_monitor_jobs SET status='cancelled',finished_at=now(),error_code='monitor_deleted'
        WHERE monitor_id=$1 AND status IN ('queued','running')").bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let user = auth::current_user(&state, &headers).await?;
    remove(&state.db, user.account_id, id).await?;
    crate::audit(
        &state.db,
        user.account_id,
        user.user_id,
        "quality_monitor.deleted",
        "quality_monitor",
        id,
        json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn enqueue_tx(
    db: &mut PgConnection,
    account_id: Uuid,
    id: Uuid,
    source: &str,
) -> Result<Uuid> {
    // Due times are compared with the database clock that schedule_due uses.
    let row=sqlx::query("SELECT q.*,c.enabled AS channel_enabled,COALESCE(q.next_run_at>now(),false) AS not_due FROM quality_monitors q JOIN channels c ON c.id=q.channel_id
        WHERE q.id=$1 AND c.account_id=$2 AND q.deleted_at IS NULL FOR UPDATE OF q")
        .bind(id).bind(account_id).fetch_optional(&mut *db).await?.ok_or(AppError::NotFound)?;
    if !row.get::<bool, _>("channel_enabled") {
        return Err(AppError::Conflict("渠道已停用，请先启用渠道".into()));
    }
    if row.get::<String, _>("kind") != "service" {
        return Err(AppError::Conflict("真伪检测请在真伪检测中发起".into()));
    }
    if source == "scheduled"
        && (!row.get::<bool, _>("schedule_enabled") || row.get::<bool, _>("not_due"))
    {
        return Err(AppError::Conflict("定时检测未到期".into()));
    }
    // Return the active job for idempotent clicks from multiple tabs or workers.
    if let Some(job)=sqlx::query_scalar::<_,Uuid>("SELECT id FROM quality_monitor_jobs WHERE monitor_id=$1 AND status IN ('queued','running')")
        .bind(id).fetch_optional(&mut *db).await? { return Ok(job); }
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM quality_monitor_jobs WHERE monitor_id=$1 AND source NOT IN ('deep','long_context') AND created_at >= date_trunc('day',now() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC'")
        .bind(id).fetch_one(&mut *db).await?;
    let limit = row.get::<i32, _>("daily_run_limit");
    if limit > 0 && count >= i64::from(limit) {
        return Err(AppError::Conflict(
            "今日检测次数已用完，将在 UTC 00:00 重置".into(),
        ));
    }
    let job=sqlx::query_scalar("INSERT INTO quality_monitor_jobs(monitor_id,config_version,source) VALUES($1,$2,$3) RETURNING id")
        .bind(id).bind(row.get::<i64,_>("config_version")).bind(source).fetch_one(&mut *db).await?;
    sqlx::query("UPDATE quality_monitors SET next_run_at=CASE WHEN schedule_enabled THEN now()+make_interval(secs=>interval_seconds) END,schedule_error=NULL WHERE id=$1")
        .bind(id).execute(&mut *db).await?;
    Ok(job)
}

pub async fn enqueue(db: &PgPool, account_id: Uuid, id: Uuid, source: &str) -> Result<Uuid> {
    let mut tx = db.begin().await?;
    let job = enqueue_tx(&mut tx, account_id, id, source).await?;
    tx.commit().await?;
    Ok(job)
}

pub async fn run(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<Value>)> {
    let user = auth::current_user(&state, &headers).await?;
    let job = enqueue(&state.db, user.account_id, id, "manual").await?;
    crate::audit(
        &state.db,
        user.account_id,
        user.user_id,
        "quality_monitor.run",
        "quality_monitor",
        id,
        json!({"job_id":job}),
    )
    .await;
    Ok((StatusCode::ACCEPTED, Json(json!({"job_id":job}))))
}
