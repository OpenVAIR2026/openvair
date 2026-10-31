use crate::edition::PRODUCT;
use crate::{
    error::{AppError, Result},
    gateway,
    state::AppState,
};
use crate::{quality_monitors, service_checks};
use axum::http::header;
use chrono::{DateTime, Duration, Utc};
use futures_util::StreamExt;
use serde_json::{Value, json};
use sqlx::Row;
use std::time::Instant;
use uuid::Uuid;

#[derive(Clone, Debug)]
struct ChannelProbe {
    protocol: String,
    base_url: String,
    secret: String,
    model: String,
}

/// One request of a service-check round.
#[derive(Debug)]
struct TestResult {
    model: String,
    tag: &'static str,
    status_code: Option<i32>,
    error_code: Option<String>,
    latency_ms: i32,
    ttfb_ms: Option<i32>,
    usage: Option<gateway::TokenUsage>,
    text: Option<String>,
    tool: Option<(String, Value)>,
    finish: Option<String>,
    /// The response carried reasoning output.
    reasoning: bool,
    stream: Option<service_checks::StreamReport>,
    /// Input tokens the request should cost by local count, for calibrated
    /// models on their native API, or why it was not counted.
    expected_input: Option<std::result::Result<f64, &'static str>>,
}

impl TestResult {
    fn unfinished(model: &str, tag: &'static str, error_code: String, latency_ms: i32) -> Self {
        Self {
            model: model.to_owned(),
            tag,
            status_code: None,
            error_code: Some(error_code),
            latency_ms,
            ttfb_ms: None,
            usage: None,
            text: None,
            tool: None,
            finish: None,
            reasoning: false,
            stream: None,
            expected_input: None,
        }
    }
}

pub(crate) fn answer_text(protocol: &str, payload: &Value) -> Option<String> {
    let text = match protocol {
        "anthropic" => payload
            .get("content")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|item| item.get("text").and_then(Value::as_str))
                    .collect::<String>()
            }),
        // Every answer part; thought parts are reasoning, not the answer.
        "gemini" => payload
            .pointer("/candidates/0/content/parts")
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .filter(|part| part["thought"] != true)
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect::<String>()
            }),
        _ => payload
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|item| item.get("message"))
            .and_then(|message| message.get("content"))
            .and_then(|content| {
                content.as_str().map(str::to_owned).or_else(|| {
                    content.as_array().map(|parts| {
                        parts
                            .iter()
                            .filter_map(|part| part.get("text").and_then(Value::as_str))
                            .collect::<String>()
                    })
                })
            }),
    };
    text.filter(|text| !text.is_empty())
}

async fn send_request(
    state: &AppState,
    probe: &ChannelProbe,
    request: &service_checks::Request,
    timeout_ms: i32,
) -> TestResult {
    let started = Instant::now();
    let mut observed_status = None;
    let mut observed_usage = None;
    let mut observed_finish = None;
    let mut observed_stream = None;
    let mut observed_reasoning = false;
    let mut expected_input = None;
    // (answer text, tool call, status code, time to first byte)
    type Answered = (Option<String>, Option<(String, Value)>, i32, i32);
    let result: Result<Answered> = async {
        if !crate::model_rules::concrete(&probe.model) {
            return Err(AppError::BadRequest("探测需要具体模型名称".into()));
        }
        let (path, body) = service_checks::body(&probe.protocol, &probe.model, request);
        expected_input = Some(crate::token_estimate::request_tokens(
            &path,
            &body,
            &probe.model,
        ));
        let target = gateway::join_base_url(&probe.base_url, &path)?;
        gateway::ensure_safe_target(&target).await?;
        let mut builder = state
            .http
            .post(target)
            .timeout(std::time::Duration::from_millis(timeout_ms as u64))
            .header("content-type", "application/json")
            .body(
                serde_json::to_vec(&body)
                    .map_err(|_| AppError::Internal("quality probe serialization failed".into()))?,
            );
        builder = match probe.protocol.as_str() {
            "anthropic" => builder
                .header("x-api-key", &probe.secret)
                .header("anthropic-version", "2023-06-01"),
            "gemini" => builder.header("x-goog-api-key", &probe.secret),
            _ => builder.header(header::AUTHORIZATION, format!("Bearer {}", probe.secret)),
        };
        let response = builder
            .send()
            .await
            .map_err(|_| AppError::BadRequest("quality_probe_transport".into()))?;
        let ttfb_ms = started.elapsed().as_millis().min(i32::MAX as u128) as i32;
        let status = response.status();
        let status_code = status.as_u16() as i32;
        observed_status = Some(status_code);
        if !status.is_success() {
            return Err(AppError::BadRequest(format!(
                "quality_probe_http_{status_code}"
            )));
        }
        let mut stream = response.bytes_stream();
        let mut body = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| AppError::BadRequest("quality_probe_body".into()))?;
            if body.len() + chunk.len() > 1024 * 1024 {
                return Err(AppError::BadRequest("quality_probe_body_too_large".into()));
            }
            body.extend_from_slice(&chunk);
        }
        if request.stream {
            // A malformed or unfinished stream is the check's finding, not an error.
            let streamed = service_checks::parse_stream(&probe.protocol, &body);
            observed_usage = gateway::parse_stream_usage(&body);
            observed_stream = Some(streamed.report);
            observed_finish = streamed.finish;
            observed_reasoning = streamed.reasoning;
            return Ok((streamed.text, None, status_code, ttfb_ms));
        }
        let payload: Value = serde_json::from_slice(&body)
            .map_err(|_| AppError::BadRequest("quality_probe_invalid_json".into()))?;
        observed_usage = gateway::parse_usage(&body);
        let text = answer_text(&probe.protocol, &payload).filter(|value| !value.trim().is_empty());
        let tool = service_checks::tool_call(&probe.protocol, &payload);
        observed_finish = service_checks::finish_reason(&probe.protocol, &payload);
        // No answer is the checks' finding: blank fails, a cap or reasoning is unfinished.
        observed_reasoning = service_checks::reasoning(&probe.protocol, &payload);
        Ok((text, tool, status_code, ttfb_ms))
    }
    .await;
    let elapsed = started.elapsed().as_millis().min(i32::MAX as u128) as i32;
    match result {
        Ok((text, tool, status_code, ttfb_ms)) => TestResult {
            model: probe.model.clone(),
            tag: request.tag,
            status_code: Some(status_code),
            error_code: None,
            latency_ms: elapsed,
            ttfb_ms: Some(ttfb_ms),
            usage: observed_usage,
            text,
            tool,
            finish: observed_finish,
            reasoning: observed_reasoning,
            stream: observed_stream,
            expected_input,
        },
        Err(error) => TestResult {
            model: probe.model.clone(),
            tag: request.tag,
            status_code: observed_status,
            error_code: Some(error.to_string()),
            latency_ms: elapsed,
            ttfb_ms: None,
            usage: observed_usage,
            text: None,
            tool: None,
            finish: observed_finish,
            reasoning: observed_reasoning,
            stream: None,
            expected_input,
        },
    }
}

pub fn start_worker(state: AppState) {
    tokio::spawn(async move {
        let mut running = tokio::task::JoinSet::new();
        loop {
            crate::system_admin::beat(&crate::system_admin::QUALITY_WORKER_BEAT);
            while running.try_join_next().is_some() {}
            if let Err(error) = schedule_due(&state).await {
                eprintln!("quality scheduler failed: {error}");
            }
            while running.len() < 4 {
                match claim_job(&state).await {
                    Ok(Some(job)) => {
                        let state = state.clone();
                        running.spawn(async move {
                            if let Err(error) = execute_job(&state, job).await {
                                eprintln!("quality job {job} failed: {error}");
                                let _ = sqlx::query("UPDATE quality_monitor_jobs SET status='failed',error_code='execution_failed',finished_at=now() WHERE id=$1 AND status='running'")
                                    .bind(job).execute(&state.db).await;
                            }
                        });
                    }
                    Ok(None) => break,
                    Err(error) => {
                        eprintln!("quality job claim failed: {error}");
                        break;
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    });
}

pub async fn schedule_due(state: &AppState) -> Result<()> {
    crate::metering::run_if_due(state);
    if crate::edition::CLOUD {
        crate::authenticity::schedule::schedule_due(state).await?;
    }
    sqlx::query("UPDATE quality_monitor_jobs SET status='failed',error_code='worker_timeout',finished_at=now()
        WHERE status='running' AND lease_until<=now()")
        .execute(&state.db).await?;
    let rows=sqlx::query("SELECT q.id,c.account_id FROM quality_monitors q JOIN channels c ON c.id=q.channel_id
        JOIN accounts a ON a.id=c.account_id AND a.disabled_at IS NULL
        WHERE q.deleted_at IS NULL AND q.kind='service' AND q.schedule_enabled AND c.enabled AND (q.next_run_at IS NULL OR q.next_run_at<=now())
        AND NOT EXISTS(SELECT 1 FROM quality_monitor_jobs j WHERE j.monitor_id=q.id AND j.status IN ('queued','running'))
        ORDER BY q.next_run_at NULLS FIRST,q.id LIMIT 20")
        .fetch_all(&state.db).await?;
    for row in rows {
        let id: Uuid = row.get("id");
        let account_id: Uuid = row.get("account_id");
        if let Err(error) = quality_monitors::enqueue(&state.db, account_id, id, "scheduled").await
        {
            // Back off invalid/budget-limited configurations without starving the queue.
            // The account predicate also protects this scheduler mutation.
            sqlx::query("UPDATE quality_monitors q SET next_run_at=now()+interval '5 minutes',schedule_error=$3 FROM channels c
                WHERE q.id=$1 AND c.id=q.channel_id AND c.account_id=$2 AND q.deleted_at IS NULL AND q.schedule_enabled
                AND (q.next_run_at IS NULL OR q.next_run_at<=now())")
                .bind(id).bind(account_id).bind(match error { AppError::Conflict(message)=>message, _=>"定时检测暂时无法排队，请手动重试".into() })
                .execute(&state.db).await?;
        }
    }
    Ok(())
}

pub async fn claim_job(state: &AppState) -> Result<Option<Uuid>> {
    let mut tx = state.db.begin().await?;
    // Lock the monitor first, as configuration changes and enqueue do. Job leases
    // survive process restarts and prevent another instance from claiming it.
    let row=sqlx::query("SELECT j.id,CASE WHEN j.source='long_context' THEN $1 ELSE q.round_timeout_ms/1000+60 END AS lease_seconds
        FROM quality_monitors q JOIN channels c ON c.id=q.channel_id
        JOIN quality_monitor_jobs j ON j.monitor_id=q.id
        WHERE j.status='queued' AND q.deleted_at IS NULL AND c.enabled AND j.config_version=q.config_version
        AND (j.source<>'scheduled' OR q.schedule_enabled)
        ORDER BY j.created_at,j.id LIMIT 1 FOR UPDATE OF q SKIP LOCKED")
        .bind(crate::long_context::LEASE_SECONDS).fetch_optional(&mut *tx).await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let id: Uuid = row.get("id");
    let changed=sqlx::query("UPDATE quality_monitor_jobs SET status='running',started_at=now(),lease_until=now()+make_interval(secs=>$2)
        WHERE id=$1 AND status='queued'")
        .bind(id).bind(row.get::<i32,_>("lease_seconds")).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok((changed.rows_affected() > 0).then_some(id))
}

/// Where and how a monitor's probes are sent.
struct ProbeSetup {
    relayed: bool,
    account_id: Uuid,
    channel_id: Uuid,
    model: String,
    upstream_model: String,
    upstream_probe: Option<ChannelProbe>,
}

impl ProbeSetup {
    fn from_row(state: &AppState, row: &sqlx::postgres::PgRow) -> Self {
        let upstream_model: String = row.get("upstream_model");
        Self {
            relayed: row.get::<String, _>("channel_kind") == "vair_key",
            account_id: row.get("account_id"),
            channel_id: row.get("channel_id"),
            model: row.get("model"),
            upstream_probe: row
                .get::<Option<String>, _>("secret_ciphertext")
                .and_then(|value| state.crypto.decrypt(&value).ok())
                .map(|secret| ChannelProbe {
                    protocol: row.get("protocol"),
                    base_url: row.get("base_url"),
                    secret,
                    model: upstream_model.clone(),
                }),
            upstream_model,
        }
    }
}

/// Sends one probe within `budget_ms`. A VAIR key channel resolves every
/// probe through its source key, so each one is checked against, and counts
/// toward, that key's limits.
async fn probe_once(
    state: &AppState,
    setup: &ProbeSetup,
    request: &service_checks::Request,
    budget_ms: u64,
    timeout_ms: i32,
) -> Result<TestResult> {
    let id = request.tag;
    let target = if setup.relayed && budget_ms > 0 {
        Some(
            gateway::relay_probe_target(state, setup.account_id, setup.channel_id, &setup.model)
                .await?,
        )
    } else {
        None
    };
    let probe = match &target {
        Some(Ok(target)) => Some(ChannelProbe {
            protocol: target.protocol.clone(),
            base_url: target.base_url.clone(),
            secret: target.secret.clone(),
            model: target.model.clone(),
        }),
        Some(Err(_)) => None,
        None if setup.relayed => None,
        None => setup.upstream_probe.clone(),
    };
    let upstream_model = &setup.upstream_model;
    let mut result = match (&target, &probe) {
        (Some(Err(reason)), _) => TestResult::unfinished(upstream_model, id, reason.clone(), 0),
        (_, Some(probe)) if budget_ms > 0 => tokio::time::timeout(
            std::time::Duration::from_millis(budget_ms),
            send_request(state, probe, request, timeout_ms),
        )
        .await
        .unwrap_or_else(|_| {
            TestResult::unfinished(
                upstream_model,
                id,
                "quality_probe_timeout".into(),
                budget_ms as i32,
            )
        }),
        (_, Some(_)) => {
            TestResult::unfinished(upstream_model, id, "quality_probe_timeout".into(), 0)
        }
        (_, None) if setup.relayed => {
            TestResult::unfinished(upstream_model, id, "quality_probe_timeout".into(), 0)
        }
        (_, None) => TestResult::unfinished(
            upstream_model,
            id,
            "quality_probe_missing_credential".into(),
            0,
        ),
    };
    if let Some(Ok(target)) = &target {
        target
            .record(
                state,
                result.status_code.map_or(502, |code| code as u16),
                (result.ttfb_ms, result.latency_ms),
                result.usage,
            )
            .await;
        // The source account's upstream model names stay private.
        result.model = upstream_model.clone();
    }
    Ok(result)
}

const MONITOR_ROW: &str = "SELECT q.*,c.account_id,c.protocol,c.base_url,c.kind AS channel_kind,j.source AS job_source,j.options AS job_options,
        COALESCE(c.model_mapping->>q.model,r.upstream_model,q.model) AS upstream_model,ck.secret_ciphertext
        FROM quality_monitor_jobs j JOIN quality_monitors q ON q.id=j.monitor_id JOIN channels c ON c.id=q.channel_id
        LEFT JOIN LATERAL(SELECT upstream_model FROM model_routes WHERE channel_id=c.id AND account_id=c.account_id
            AND public_model=q.model AND protocol=c.protocol AND enabled ORDER BY priority,id LIMIT 1) r ON true
        LEFT JOIN LATERAL(SELECT secret_ciphertext FROM channel_keys WHERE channel_id=c.id AND enabled ORDER BY created_at LIMIT 1) ck ON true
        WHERE j.id=$1 AND j.status='running' AND j.config_version=q.config_version AND q.deleted_at IS NULL AND c.enabled";

pub async fn execute_job(state: &AppState, job_id: Uuid) -> Result<()> {
    // Authenticity monitors share the queue and lease but run their own probes.
    let kind: Option<String> = sqlx::query_scalar(
        "SELECT q.kind FROM quality_monitor_jobs j JOIN quality_monitors q ON q.id=j.monitor_id WHERE j.id=$1",
    )
    .bind(job_id)
    .fetch_optional(&state.db)
    .await?;
    if kind.as_deref() == Some("authenticity") {
        return crate::authenticity::deep::execute(state, job_id).await;
    }
    let row = sqlx::query(MONITOR_ROW)
        .bind(job_id)
        .fetch_optional(&state.db)
        .await?;
    let Some(row) = row else {
        return Ok(());
    };
    if row.get::<String, _>("job_source") == "long_context" {
        return execute_long_context(state, job_id, &row).await;
    }
    let monitor_id: Uuid = row.get("id");
    let channel_id: Uuid = row.get("channel_id");
    let account_id: Uuid = row.get("account_id");
    let version: i64 = row.get("config_version");
    let model: String = row.get("model");
    let upstream_model: String = row.get("upstream_model");
    let methods: Vec<String> = serde_json::from_value(row.get("methods")).unwrap_or_default();
    // Fresh random parameters every round, so answers cannot be cached or matched.
    let round = service_checks::plan(&methods, &mut rand::thread_rng());
    let timeout_ms: i32 = row.get("timeout_ms");
    let round_timeout_ms: i32 = row.get("round_timeout_ms");
    let started_at = Utc::now();
    let started = Instant::now();
    let setup = ProbeSetup::from_row(state, &row);
    let mut results = Vec::new();
    for request in &round.requests {
        let active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM quality_monitor_jobs j JOIN quality_monitors q ON q.id=j.monitor_id
            JOIN channels c ON c.id=q.channel_id WHERE j.id=$1 AND j.status='running' AND j.lease_until>now()
            AND q.config_version=$2 AND q.deleted_at IS NULL AND c.account_id=$3 AND c.enabled)")
            .bind(job_id).bind(version).bind(account_id).fetch_one(&state.db).await?;
        if !active {
            break;
        }
        let remaining =
            (i64::from(round_timeout_ms) - started.elapsed().as_millis() as i64).max(0) as u64;
        let result = probe_once(
            state,
            &setup,
            request,
            remaining.min(timeout_ms as u64),
            timeout_ms,
        )
        .await?;
        results.push(result);
    }
    let finished_at = Utc::now();
    let replies: Vec<service_checks::Reply> = round
        .requests
        .iter()
        .map(
            |request| match results.iter().find(|r| r.tag == request.tag) {
                Some(r) => service_checks::Reply {
                    text: r.text.clone(),
                    tool: r.tool.clone(),
                    input_tokens: r.usage.map(|u| u.input()),
                    output_tokens: r.usage.map(|u| u.output()),
                    finish: r.finish.clone(),
                    reasoning: r.reasoning,
                    stream: r.stream,
                    expected_input: r.expected_input.and_then(std::result::Result::ok),
                    uncounted: r.expected_input.and_then(std::result::Result::err),
                    error: r.error_code.clone(),
                },
                None => service_checks::Reply {
                    error: Some("quality_probe_not_run".into()),
                    ..Default::default()
                },
            },
        )
        .collect();
    let mut judged = service_checks::judge(&round, &replies);
    // The previous metering result decides whether an estimate failure is
    // confirmed and whether the counting method changed since.
    let previous_metering: Option<Value> = sqlx::query_scalar(
        "SELECT t->'detail' FROM channel_monitor_runs r CROSS JOIN LATERAL jsonb_array_elements(r.details->'tests') t
         WHERE r.monitor_id=$1 AND r.kind='quality' AND r.details->>'applied'='true' AND t->>'id'='metering'
         ORDER BY r.started_at DESC LIMIT 1",
    )
    .bind(monitor_id)
    .fetch_optional(&state.db)
    .await?;
    let previous_method = previous_metering
        .as_ref()
        .map(|d| d["method"].as_str().unwrap_or("estimate").to_owned());
    if let Some(metering) = judged.iter_mut().find(|j| j.id == "metering")
        && let (Some(previous), Some(method)) =
            (&previous_method, metering.detail["method"].as_str())
        && previous != method
    {
        metering.detail["method_changed"] = json!(true);
    }
    let total = judged.len();
    let completed = judged.iter().filter(|j| j.passed.is_some()).count();
    let passed = judged.iter().filter(|j| j.passed == Some(true)).count();
    // Unfinished side checks only warn: once the critical checks are judged,
    // the score covers the checks that finished.
    let conclusive = completed > 0
        && judged
            .iter()
            .filter(|j| j.critical)
            .all(|j| j.passed.is_some());
    let score = conclusive.then(|| passed as f64 / completed as f64 * 100.0);
    // Tampering or a replayed answer is conclusive even when other checks did not finish.
    let critical_failure = judged.iter().any(|j| j.critical && j.passed == Some(false));
    let observation = match score {
        _ if critical_failure => "degraded",
        Some(s) if s >= row.get::<f64, _>("min_score") => "healthy",
        Some(_) => "degraded",
        None => "unknown",
    };
    let error = results.iter().find_map(|r| r.error_code.clone());
    let mut tx = state.db.begin().await?;
    let current=sqlx::query("SELECT q.status,q.consecutive_failures,q.config_version,q.deleted_at,c.enabled,
        q.interval_seconds,q.schedule_enabled FROM quality_monitors q JOIN channels c ON c.id=q.channel_id
        WHERE q.id=$1 AND c.account_id=$2 FOR UPDATE OF q")
        .bind(monitor_id).bind(account_id).fetch_optional(&mut *tx).await?;
    let Some(current) = current else {
        return Ok(());
    };
    let job_active:bool=sqlx::query_scalar("SELECT status='running' AND lease_until>now() FROM quality_monitor_jobs WHERE id=$1 FOR UPDATE")
        .bind(job_id).fetch_one(&mut *tx).await?;
    let applied = job_active
        && current.get::<i64, _>("config_version") == version
        && current
            .get::<Option<DateTime<Utc>>, _>("deleted_at")
            .is_none()
        && current.get::<bool, _>("enabled");
    let tests: Vec<Value> = judged
        .iter()
        .map(|j| {
            let used: Vec<&TestResult> = results.iter().filter(|r| j.tags.contains(&r.tag)).collect();
            json!({"id":j.id,"model":upstream_model,"passed":j.passed == Some(true),"critical":j.critical,
                "latency_ms":used.iter().map(|r| r.latency_ms).sum::<i32>(),
                "status_code":used.iter().rev().find_map(|r| r.status_code),
                "error_code":j.error,"detail":j.detail})
        })
        .collect();
    let details = json!({"suite_version":service_checks::SUITE_VERSION,"config_version":version,"public_model":model,"upstream_model":upstream_model,
        "job_id":job_id,"applied":applied,"passed":passed,"completed":completed,"total":total,"tests":tests});
    sqlx::query("INSERT INTO channel_monitor_runs(channel_id,monitor_id,kind,model,status,score,latency_ms,ttfb_ms,status_code,error_code,details,started_at,finished_at)
        VALUES($1,$2,'quality',$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)")
        .bind(channel_id).bind(monitor_id).bind(&model).bind(observation).bind(score)
        .bind(started.elapsed().as_millis().min(i32::MAX as u128) as i32).bind(results.first().and_then(|r|r.ttfb_ms))
        .bind(results.iter().rev().find_map(|r|r.status_code)).bind(&error).bind(details).bind(started_at).bind(finished_at)
        .execute(&mut *tx).await?;
    let mut newly_degraded = false;
    if applied {
        let previous_status: String = current.get("status");
        let previous_failures: i32 = current.get("consecutive_failures");
        let failures = match observation {
            "degraded" => previous_failures.saturating_add(1),
            "healthy" => 0,
            _ => previous_failures,
        };
        let status = match observation {
            "healthy" => "healthy",
            "degraded" if failures >= row.get::<i32, _>("failure_threshold") => "degraded",
            _ => previous_status.as_str(),
        };
        newly_degraded = status == "degraded" && previous_status != "degraded";
        let interval: i32 = current.get("interval_seconds");
        let valid_until =
            conclusive.then(|| finished_at + Duration::seconds(i64::from(interval) * 2));
        sqlx::query("UPDATE quality_monitors SET status=$2,last_observation=$3,score=$4,consecutive_failures=$5,last_error=$6,
            checked_at=$7,valid_until=COALESCE($8,valid_until),
            next_run_at=CASE WHEN schedule_enabled THEN $7+make_interval(secs=>$9) END,updated_at=now() WHERE id=$1")
            .bind(monitor_id).bind(status).bind(observation).bind(score).bind(failures).bind(&error).bind(finished_at).bind(valid_until)
            .bind(if observation=="unknown" {interval.min(300)} else {interval}).execute(&mut *tx).await?;
    }
    sqlx::query("UPDATE quality_monitor_jobs SET status=$2,error_code=$3,finished_at=now() WHERE id=$1 AND status='running'")
        .bind(job_id).bind(if applied {"completed"} else {"cancelled"}).bind(if applied {error.as_deref()} else {Some("monitor_changed")})
        .execute(&mut *tx).await?;
    tx.commit().await?;
    if applied
        && let Some(metering) = judged
            .iter()
            .find(|j| j.id == "metering" && j.passed.is_some())
    {
        let previous_failed = previous_metering
            .as_ref()
            .is_some_and(|d| d["hidden_prompt"] == true || d["inflated"] == true);
        crate::metering::settle_check(
            state,
            account_id,
            channel_id,
            &model,
            &upstream_model,
            &metering.detail,
            previous_failed,
        )
        .await?;
    }
    if newly_degraded {
        let failed: Vec<&str> = judged
            .iter()
            .filter(|j| j.passed == Some(false))
            .map(|j| service_checks::label(j.id))
            .collect();
        let channel: String = sqlx::query_scalar("SELECT name FROM channels WHERE id=$1")
            .bind(channel_id)
            .fetch_one(&state.db)
            .await?;
        crate::email::account_alert(
            state,
            account_id,
            crate::email::AlertKind::Service,
            "服务体检告警",
            format!(
                "渠道「{channel}」的模型 {model} 的服务体检已确认异常。\n未通过：{}\n\n请登录 {PRODUCT} 在「渠道监控 → 服务体检」中查看。",
                if failed.is_empty() { "得分低于最低通过分数".to_owned() } else { failed.join("、") }
            ),
        )
        .await?;
    }
    Ok(())
}

/// Runs the long-context tiers in order up to the chosen one and stops at
/// the first that is not passed. Results go to the service check's records
/// but never change its status, score or routing.
async fn execute_long_context(
    state: &AppState,
    job_id: Uuid,
    row: &sqlx::postgres::PgRow,
) -> Result<()> {
    use crate::long_context;
    let setup = ProbeSetup::from_row(state, row);
    let monitor_id: Uuid = row.get("id");
    let version: i64 = row.get("config_version");
    let max_tokens = row
        .get::<Option<Value>, _>("job_options")
        .and_then(|o| o["max_tokens"].as_i64())
        .unwrap_or(i64::from(long_context::TIERS[0])) as i32;
    let started_at = Utc::now();
    let started = Instant::now();
    let mut tiers = Vec::new();
    let mut outcomes = Vec::new();
    for tier in long_context::TIERS.into_iter().filter(|t| *t <= max_tokens) {
        let active: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM quality_monitor_jobs WHERE id=$1 AND status='running' AND lease_until>now())",
        )
        .bind(job_id)
        .fetch_one(&state.db)
        .await?;
        if !active {
            break;
        }
        let probe = long_context::plan(tier, &mut rand::thread_rng());
        let result = probe_once(
            state,
            &setup,
            &long_context::request(&probe),
            long_context::REQUEST_TIMEOUT_MS as u64,
            long_context::REQUEST_TIMEOUT_MS,
        )
        .await?;
        let text = result.text.as_deref().unwrap_or("");
        let unanswered = (result.error_code.is_none() && text.trim().is_empty())
            .then(|| service_checks::unanswered(result.finish.as_deref(), result.reasoning))
            .flatten();
        let error_code = result
            .error_code
            .clone()
            .or_else(|| unanswered.map(str::to_owned));
        let passed = error_code
            .is_none()
            .then(|| text.contains(&probe.passphrase));
        tiers.push(json!({"tokens":tier,"input_tokens":result.usage.map(|u| u.input()),
            "passed":passed,"latency_ms":result.latency_ms,"status_code":result.status_code,"error_code":error_code}));
        outcomes.push((tier, passed));
        if passed != Some(true) {
            break;
        }
    }
    let (max_passed, status) = long_context::summarize(&outcomes);
    let finished = outcomes.iter().any(|(_, passed)| passed.is_some());
    let error = tiers
        .iter()
        .find_map(|t| t["error_code"].as_str().map(str::to_owned));
    let details = json!({"requested":max_tokens,"max_passed":max_passed,"tiers":tiers,"config_version":version,
        "public_model":setup.model,"upstream_model":setup.upstream_model,"job_id":job_id});
    let mut tx = state.db.begin().await?;
    sqlx::query("INSERT INTO channel_monitor_runs(channel_id,monitor_id,kind,model,status,latency_ms,error_code,details,started_at,finished_at)
        VALUES($1,$2,'long_context',$3,$4,$5,$6,$7,$8,now())")
        .bind(setup.channel_id).bind(monitor_id).bind(&setup.model).bind(status)
        .bind(started.elapsed().as_millis().min(i32::MAX as u128) as i32).bind(&error).bind(details).bind(started_at)
        .execute(&mut *tx).await?;
    if finished {
        sqlx::query("UPDATE quality_monitors SET long_context_max=$2,long_context_checked_at=now() WHERE id=$1")
            .bind(monitor_id)
            .bind(max_passed)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE quality_monitor_jobs SET status='completed',error_code=$2,finished_at=now() WHERE id=$1 AND status='running'")
        .bind(job_id)
        .bind(&error)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
#[path = "monitor_db_tests.rs"]
mod db_tests;
