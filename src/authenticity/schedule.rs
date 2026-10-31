//! Settings and scheduling of authenticity monitors: a claimed model, a few
//! samples per scheduled run, and a rolling window of samples that is judged
//! after every run. The window verdict may steer routing and sends alerts.
use super::{baseline, deep};
use crate::edition::PRODUCT;
use crate::{
    email,
    error::{AppError, Result},
    state::AppState,
};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AuthenticitySettings {
    pub claimed_model: Option<String>,
    pub schedule_enabled: bool,
    pub interval_seconds: i32,
    pub samples_per_run: i32,
    pub window_days: i32,
    pub daily_sample_limit: i32,
    pub route_policy: String,
}

impl AuthenticitySettings {
    fn validate(&self) -> Result<()> {
        if !(3600..=86400).contains(&self.interval_seconds)
            || !(1..=8).contains(&self.samples_per_run)
            || !(1..=30).contains(&self.window_days)
            || !(0..=100_000).contains(&self.daily_sample_limit)
        {
            return Err(AppError::BadRequest(
                "抽样周期需在 1–24 小时，每轮 1–8 个样本，窗口 1–30 天".into(),
            ));
        }
        if !["observe", "deprioritize", "exclude"].contains(&self.route_policy.as_str()) {
            return Err(AppError::BadRequest("无效的路由策略".into()));
        }
        if self.schedule_enabled && self.claimed_model.is_none() {
            return Err(AppError::BadRequest("开启定时抽样前请选择申报模型".into()));
        }
        Ok(())
    }
}

/// Applies a partial settings change to an authenticity monitor. Changing the
/// claim or what a window means starts a fresh window and cancels runs in flight.
pub async fn patch(
    db: &PgPool,
    account_id: Uuid,
    user_id: Uuid,
    id: Uuid,
    patch: &Value,
) -> Result<AuthenticitySettings> {
    let mut patch = patch
        .as_object()
        .cloned()
        .ok_or_else(|| AppError::BadRequest("监控配置必须是 JSON 对象".into()))?;
    let acknowledge = patch.remove("acknowledge_risk").and_then(|v| v.as_bool()) == Some(true);
    let mut tx = db.begin().await?;
    let row = sqlx::query(
        "SELECT q.*,c.authenticity_ack_at IS NOT NULL AS acknowledged FROM quality_monitors q
         JOIN channels c ON c.id=q.channel_id
         WHERE q.id=$1 AND c.account_id=$2 AND q.kind='authenticity' AND q.deleted_at IS NULL FOR UPDATE OF q,c",
    )
    .bind(id)
    .bind(account_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    let current = AuthenticitySettings {
        claimed_model: row.get("claimed_model"),
        schedule_enabled: row.get("schedule_enabled"),
        interval_seconds: row.get("interval_seconds"),
        samples_per_run: row.get("samples_per_run"),
        window_days: row.get("window_days"),
        daily_sample_limit: row.get("daily_sample_limit"),
        route_policy: row.get("route_policy"),
    };
    let mut merged = json!(current);
    for (key, value) in patch {
        merged[key] = value;
    }
    let settings: AuthenticitySettings = serde_json::from_value(merged)
        .map_err(|_| AppError::BadRequest("真伪检测配置字段无效".into()))?;
    settings.validate()?;
    if let Some(claimed) = &settings.claimed_model {
        let active: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM model_baseline_versions WHERE model=$1 AND status='active')",
        )
        .bind(claimed)
        .fetch_one(&mut *tx)
        .await?;
        if !active {
            return Err(AppError::BadRequest(format!("{claimed} 还没有启用的基准")));
        }
    }
    // Scheduled probes may be noticed by the relay, so they need the same acknowledgement.
    if settings.schedule_enabled && !row.get::<bool, _>("acknowledged") {
        if !acknowledge {
            return Err(AppError::Refused(
                "authenticity_ack_required",
                "开启定时抽样前需要确认风险提示".into(),
            ));
        }
        sqlx::query(
            "UPDATE channels SET authenticity_ack_at=now(),authenticity_ack_by=$2 WHERE id=$1",
        )
        .bind(row.get::<Uuid, _>("channel_id"))
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    }
    let reset = settings.claimed_model != current.claimed_model
        || settings.window_days != current.window_days;
    sqlx::query(
        "UPDATE quality_monitors SET claimed_model=$2,schedule_enabled=$3,interval_seconds=$4,samples_per_run=$5,
         window_days=$6,daily_sample_limit=$7,route_policy=$8,schedule_error=NULL,
         config_version=config_version+CASE WHEN $9 THEN 1 ELSE 0 END,
         window_verdict=CASE WHEN $9 THEN NULL ELSE window_verdict END,
         window_evidence=CASE WHEN $9 THEN NULL ELSE window_evidence END,
         window_checked_at=CASE WHEN $9 THEN NULL ELSE window_checked_at END,
         window_valid_until=CASE WHEN $9 THEN NULL ELSE window_valid_until END,
         alerted_verdict=CASE WHEN $9 THEN NULL ELSE alerted_verdict END,
         next_run_at=CASE WHEN NOT $3 THEN NULL WHEN $9 OR NOT schedule_enabled OR interval_seconds<>$4 THEN now()
             ELSE COALESCE(next_run_at,now()) END,
         updated_at=now() WHERE id=$1",
    )
    .bind(id)
    .bind(&settings.claimed_model)
    .bind(settings.schedule_enabled)
    .bind(settings.interval_seconds)
    .bind(settings.samples_per_run)
    .bind(settings.window_days)
    .bind(settings.daily_sample_limit)
    .bind(&settings.route_policy)
    .bind(reset)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE quality_monitor_jobs SET status='cancelled',finished_at=now(),error_code='monitor_changed'
         WHERE monitor_id=$1 AND status IN ('queued','running') AND ($2 OR (source='scheduled' AND NOT $3))",
    )
    .bind(id)
    .bind(reset)
    .bind(settings.schedule_enabled)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(settings)
}

/// Fills the claimed model from the channel's upstream (or public) model name.
pub async fn set_default_claim(db: &PgPool, account_id: Uuid, id: Uuid) -> Result<()> {
    let names: Option<(String, String)> = sqlx::query_as(
        "SELECT q.model,COALESCE(c.model_mapping->>q.model,r.upstream_model,q.model)
         FROM quality_monitors q JOIN channels c ON c.id=q.channel_id
         LEFT JOIN LATERAL (SELECT upstream_model FROM model_routes WHERE channel_id=c.id AND account_id=$2
             AND public_model=q.model AND protocol=c.protocol AND enabled ORDER BY priority,id LIMIT 1) r ON true
         WHERE q.id=$1 AND c.account_id=$2",
    )
    .bind(id)
    .bind(account_id)
    .fetch_optional(db)
    .await?;
    let Some((public, upstream)) = names else {
        return Ok(());
    };
    let models: Vec<String> = baseline::active_baselines(db)
        .await?
        .into_iter()
        .map(|b| b.model)
        .collect();
    if let Some(claim) = deep::default_claim(&[&upstream, &public], &models) {
        sqlx::query(
            "UPDATE quality_monitors SET claimed_model=$2 WHERE id=$1 AND claimed_model IS NULL",
        )
        .bind(id)
        .bind(claim)
        .execute(db)
        .await?;
    }
    Ok(())
}

/// Queues one scheduled sampling run.
pub async fn enqueue_scheduled(db: &PgPool, account_id: Uuid, id: Uuid) -> Result<Uuid> {
    let mut tx = db.begin().await?;
    let row = sqlx::query(
        "SELECT q.*,c.enabled AS channel_enabled,c.authenticity_ack_at IS NOT NULL AS acknowledged,
           COALESCE(q.next_run_at>now(),false) AS not_due
         FROM quality_monitors q JOIN channels c ON c.id=q.channel_id
         WHERE q.id=$1 AND c.account_id=$2 AND q.kind='authenticity' AND q.deleted_at IS NULL FOR UPDATE OF q",
    )
    .bind(id)
    .bind(account_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    if !row.get::<bool, _>("channel_enabled")
        || !row.get::<bool, _>("schedule_enabled")
        || row.get::<bool, _>("not_due")
    {
        return Err(AppError::Conflict("定时抽样未到期".into()));
    }
    if !row.get::<bool, _>("acknowledged") {
        return Err(AppError::Conflict("需要先确认真伪检测的风险提示".into()));
    }
    let claimed: Option<String> = row.get("claimed_model");
    let claimed = claimed.ok_or_else(|| AppError::Conflict("请先选择申报模型".into()))?;
    let baseline: Option<(Uuid, i32)> = sqlx::query_as(
        "SELECT id,pool_version FROM model_baseline_versions WHERE model=$1 AND status='active'",
    )
    .bind(&claimed)
    .fetch_optional(&mut *tx)
    .await?;
    let (baseline_id, pool_version) =
        baseline.ok_or_else(|| AppError::Conflict(format!("{claimed} 的基准已停用")))?;
    if let Some(job) = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM quality_monitor_jobs WHERE monitor_id=$1 AND status IN ('queued','running')",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    {
        return Ok(job);
    }
    let samples: i32 = row.get("samples_per_run");
    let limit: i32 = row.get("daily_sample_limit");
    if limit > 0 {
        let used: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authenticity_samples WHERE monitor_id=$1
             AND created_at >= date_trunc('day',now() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC'",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        if used + i64::from(samples) > i64::from(limit) {
            return Err(AppError::Conflict(
                "今日样本额度已用完，将在 UTC 00:00 重置".into(),
            ));
        }
    }
    let job: Uuid = sqlx::query_scalar(
        "INSERT INTO quality_monitor_jobs(monitor_id,config_version,source,claimed_model,options,target_samples)
         VALUES($1,$2,'scheduled',$3,$4,$5) RETURNING id",
    )
    .bind(id)
    .bind(row.get::<i64, _>("config_version"))
    .bind(&claimed)
    .bind(json!({"early_stop":false,"concurrency":1,"baseline_version_id":baseline_id,
        "pool_version":pool_version,"scheduled":true}))
    .bind(samples)
    .fetch_one(&mut *tx)
    .await?;
    // A little jitter keeps scheduled probes from forming an obvious pattern.
    let interval: i32 = row.get("interval_seconds");
    let jitter = rand::thread_rng().gen_range(-(interval / 10)..=interval / 10);
    sqlx::query(
        "UPDATE quality_monitors SET next_run_at=now()+make_interval(secs=>$2),schedule_error=NULL WHERE id=$1",
    )
    .bind(id)
    .bind(interval + jitter)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(job)
}

/// Queues runs of due authenticity monitors; failures back off with a reason.
pub async fn schedule_due(state: &AppState) -> Result<()> {
    let rows = sqlx::query(
        "SELECT q.id,c.account_id FROM quality_monitors q JOIN channels c ON c.id=q.channel_id
         JOIN accounts a ON a.id=c.account_id AND a.disabled_at IS NULL
         WHERE q.deleted_at IS NULL AND q.kind='authenticity' AND q.schedule_enabled AND c.enabled
           AND (q.next_run_at IS NULL OR q.next_run_at<=now())
           AND NOT EXISTS(SELECT 1 FROM quality_monitor_jobs j WHERE j.monitor_id=q.id AND j.status IN ('queued','running'))
         ORDER BY q.next_run_at NULLS FIRST,q.id LIMIT 20",
    )
    .fetch_all(&state.db)
    .await?;
    for row in rows {
        let id: Uuid = row.get("id");
        let account_id: Uuid = row.get("account_id");
        if let Err(error) = enqueue_scheduled(&state.db, account_id, id).await {
            sqlx::query(
                "UPDATE quality_monitors q SET next_run_at=now()+interval '30 minutes',schedule_error=$3 FROM channels c
                 WHERE q.id=$1 AND c.id=q.channel_id AND c.account_id=$2 AND q.deleted_at IS NULL AND q.schedule_enabled",
            )
            .bind(id)
            .bind(account_id)
            .bind(match error {
                AppError::Conflict(message) => message,
                _ => "定时抽样暂时无法排队".into(),
            })
            .execute(&state.db)
            .await?;
        }
    }
    Ok(())
}

/// Emails the account when the window verdict newly becomes a mismatch.
pub async fn alert(
    state: &AppState,
    monitor_id: Uuid,
    verdict: &str,
    evidence: &Value,
) -> Result<()> {
    let row = sqlx::query(
        "SELECT q.model,q.claimed_model,q.alerted_verdict,c.name AS channel_name,c.account_id
         FROM quality_monitors q JOIN channels c ON c.id=q.channel_id WHERE q.id=$1",
    )
    .bind(monitor_id)
    .fetch_optional(&state.db)
    .await?;
    let Some(row) = row else {
        return Ok(());
    };
    let mismatch = matches!(verdict, "suspected" | "confirmed");
    let previous: Option<String> = row.get("alerted_verdict");
    if !mismatch {
        if previous.is_some() {
            sqlx::query("UPDATE quality_monitors SET alerted_verdict=NULL WHERE id=$1")
                .bind(monitor_id)
                .execute(&state.db)
                .await?;
        }
        return Ok(());
    }
    if previous.as_deref() == Some(verdict) {
        return Ok(());
    }
    // The verdict is recorded even when the email is off or unreachable.
    sqlx::query("UPDATE quality_monitors SET alerted_verdict=$2 WHERE id=$1")
        .bind(monitor_id)
        .bind(verdict)
        .execute(&state.db)
        .await?;
    let label = if verdict == "confirmed" {
        "确认不符"
    } else {
        "疑似不符"
    };
    let best = evidence["best_model"].as_str().unwrap_or("未知");
    email::account_alert(
        state,
        row.get("account_id"),
        email::AlertKind::Authenticity,
        "真伪检测告警",
        format!(
            "渠道「{}」的模型 {} 在真伪检测的定时抽样中被判为「{label}」。\n申报模型：{}\n数字分布最像：{best}\n有效样本：{}\n\n请登录 {PRODUCT} 在「渠道监控 → 真伪检测」中查看报告。",
            row.get::<String, _>("channel_name"),
            row.get::<String, _>("model"),
            row.get::<Option<String>, _>("claimed_model")
                .unwrap_or_default(),
            evidence["valid_samples"],
        ),
    )
    .await
}
