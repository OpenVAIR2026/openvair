//! Super-admin maintenance of the probe prompt pool and imported model baselines.
//!
//! Baselines are collected off-server from a plan and imported as batches.
//! A version fixes the collection conditions; batches with the same
//! conditions merge incrementally, each merge or exclusion bumps the revision
//! and recomputes the version's statistics from its included batches.
use super::{
    bundle,
    evaluate::{self, MIN_EVAL_SAMPLES, ModelData},
    fingerprint::{self, Counts},
};
use crate::{
    audit, auth,
    error::{AppError, Result},
    pricing,
    state::AppState,
};
use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, PgConnection, PgPool, Row};
use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

/// Stage one supports a single value range so baselines stay comparable.
pub const RANGE_MAX: u16 = 355;
/// Reasoning modes: "off" turns reasoning off; "default" sends no reasoning
/// parameter, for models that reject turning it off.
pub const REASONING_MODES: [&str; 2] = ["off", "default"];
pub const SOURCE: &str = "openrouter";
const PLAN_DAYS: i64 = 14;
/// With reasoning off, more reasoning tokens than this mean the provider ignored it.
pub const MAX_REASONING_TOKENS: i64 = 20;
/// Rough per-sample usage for estimates before a baseline exists.
const FINGERPRINT_TOKENS: (i64, i64) = (250, 1200);
const TOKEN_TOKENS: (i64, i64) = (150, 60);

/// Statistics of a version, recomputed from its included batches.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Stats {
    pub fingerprint: Option<Counts>,
    /// Median input tokens per token-probe template.
    #[serde(default)]
    pub token: BTreeMap<Uuid, f64>,
    #[serde(default)]
    pub avg_input: f64,
    #[serde(default)]
    pub avg_output: f64,
}

fn invalid(message: &str) -> AppError {
    AppError::BadRequest(message.into())
}

// ---------------------------------------------------------------- prompt pool

#[derive(Serialize)]
struct TemplateView {
    id: Uuid,
    probe_kind: String,
    content: String,
    created_at: DateTime<Utc>,
    retired_at: Option<DateTime<Utc>>,
}

#[derive(Serialize, FromRow)]
struct PoolVersionView {
    version: i32,
    template_ids: Vec<Uuid>,
    published_at: DateTime<Utc>,
}

pub async fn list_templates(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    auth::require_super_admin(&state, &headers).await?;
    let rows = sqlx::query(
        "SELECT id,probe_kind,content_ciphertext,created_at,retired_at FROM probe_templates ORDER BY created_at,id",
    )
    .fetch_all(&state.db)
    .await?;
    let templates = rows
        .into_iter()
        .map(|row| {
            Ok(TemplateView {
                id: row.get("id"),
                probe_kind: row.get("probe_kind"),
                content: state
                    .crypto
                    .decrypt(&row.get::<String, _>("content_ciphertext"))
                    .map_err(|_| AppError::Internal("probe template decrypt failed".into()))?,
                created_at: row.get("created_at"),
                retired_at: row.get("retired_at"),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let versions: Vec<PoolVersionView> = sqlx::query_as(
        "SELECT version,template_ids,published_at FROM probe_pool_versions ORDER BY version DESC",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({"templates":templates,"versions":versions})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateInput {
    probe_kind: String,
    content: String,
}

pub fn validate_template(kind: &str, content: &str) -> Result<()> {
    let length = content.chars().count();
    if !(10..=4000).contains(&length) {
        return Err(invalid("模板长度需要在 10 到 4000 个字符之间"));
    }
    match kind {
        "fingerprint" if content.contains("{n}") && content.contains("{r}") => Ok(()),
        "fingerprint" => Err(invalid(
            "指纹模板需要包含 {n}（数量）和 {r}（取值上限）占位符",
        )),
        "token" if !content.contains('{') => Ok(()),
        "token" => Err(invalid("Tokenizer 模板是固定文本，不能包含占位符")),
        _ => Err(invalid("探针类型必须是 fingerprint 或 token")),
    }
}

pub async fn create_template(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<TemplateInput>,
) -> Result<(StatusCode, Json<Value>)> {
    let admin = auth::require_super_admin(&state, &headers).await?;
    let content = input.content.trim();
    validate_template(&input.probe_kind, content)?;
    let ciphertext = state
        .crypto
        .encrypt(content)
        .map_err(|_| AppError::Internal("probe template encrypt failed".into()))?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO probe_templates(probe_kind,content_ciphertext,created_by) VALUES($1,$2,$3) RETURNING id",
    )
    .bind(&input.probe_kind)
    .bind(ciphertext)
    .bind(admin.user_id)
    .fetch_one(&state.db)
    .await?;
    // Template text is secret and never written to the audit log.
    audit(
        &state.db,
        admin.account_id,
        admin.user_id,
        "platform.probe_template_created",
        "probe_template",
        id,
        json!({"probe_kind":input.probe_kind}),
    )
    .await;
    Ok((StatusCode::CREATED, Json(json!({"id":id}))))
}

/// Adds the built-in default templates that are not in the pool yet. A
/// default an admin already retired is not added again.
pub async fn import_default_templates(state: &AppState, user_id: Uuid) -> Result<usize> {
    let existing: Vec<String> =
        sqlx::query_scalar("SELECT content_ciphertext FROM probe_templates")
            .fetch_all(&state.db)
            .await?
            .into_iter()
            .filter_map(|c: String| state.crypto.decrypt(&c).ok())
            .collect();
    let mut added = 0;
    for (kind, content) in super::defaults::all() {
        if existing.iter().any(|e| e == content) {
            continue;
        }
        let ciphertext = state
            .crypto
            .encrypt(content)
            .map_err(|_| AppError::Internal("probe template encrypt failed".into()))?;
        sqlx::query(
            "INSERT INTO probe_templates(probe_kind,content_ciphertext,created_by) VALUES($1,$2,$3)",
        )
        .bind(kind)
        .bind(ciphertext)
        .bind(user_id)
        .execute(&state.db)
        .await?;
        added += 1;
    }
    Ok(added)
}

pub async fn import_defaults(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let admin = auth::require_super_admin(&state, &headers).await?;
    let added = import_default_templates(&state, admin.user_id).await?;
    audit(
        &state.db,
        admin.account_id,
        admin.user_id,
        "platform.probe_templates_defaults_imported",
        "probe_template",
        admin.user_id,
        json!({"added":added}),
    )
    .await;
    Ok(Json(json!({"added":added})))
}

pub async fn retire_template(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let admin = auth::require_super_admin(&state, &headers).await?;
    let n = sqlx::query(
        "UPDATE probe_templates SET retired_at=now() WHERE id=$1 AND retired_at IS NULL",
    )
    .bind(id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(AppError::NotFound);
    }
    audit(
        &state.db,
        admin.account_id,
        admin.user_id,
        "platform.probe_template_retired",
        "probe_template",
        id,
        json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn publish_pool_version(db: &PgPool, user_id: Uuid) -> Result<i32> {
    let mut tx = db.begin().await?;
    sqlx::query("LOCK TABLE probe_pool_versions IN EXCLUSIVE MODE")
        .execute(&mut *tx)
        .await?;
    let rows = sqlx::query(
        "SELECT id,probe_kind FROM probe_templates WHERE retired_at IS NULL ORDER BY created_at,id",
    )
    .fetch_all(&mut *tx)
    .await?;
    let count = |kind: &str| {
        rows.iter()
            .filter(|r| r.get::<String, _>("probe_kind") == kind)
            .count()
    };
    if count("fingerprint") < 1 || count("token") < 2 {
        return Err(AppError::Conflict(
            "发布需要至少 1 个指纹模板和 2 个 Tokenizer 模板".into(),
        ));
    }
    let ids: Vec<Uuid> = rows.iter().map(|r| r.get("id")).collect();
    let version: i32 = sqlx::query_scalar(
        "INSERT INTO probe_pool_versions(version,template_ids,published_by)
         SELECT COALESCE(max(version),0)+1,$1,$2 FROM probe_pool_versions RETURNING version",
    )
    .bind(&ids)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(version)
}

pub async fn publish_pool(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<Value>)> {
    let admin = auth::require_super_admin(&state, &headers).await?;
    let version = publish_pool_version(&state.db, admin.user_id).await?;
    audit(
        &state.db,
        admin.account_id,
        admin.user_id,
        "platform.probe_pool_published",
        "probe_pool_version",
        admin.user_id,
        json!({"version":version}),
    )
    .await;
    Ok((StatusCode::CREATED, Json(json!({"version":version}))))
}

pub struct PoolTemplate {
    pub id: Uuid,
    pub probe_kind: String,
    pub content: String,
}

/// Decrypted templates of a published pool version, in snapshot order.
pub async fn pool_templates(state: &AppState, version: i32) -> Result<Vec<PoolTemplate>> {
    let rows = sqlx::query(
        "SELECT t.id,t.probe_kind,t.content_ciphertext FROM probe_pool_versions v
         CROSS JOIN LATERAL unnest(v.template_ids) WITH ORDINALITY AS u(id,ord)
         JOIN probe_templates t ON t.id=u.id WHERE v.version=$1 ORDER BY u.ord",
    )
    .bind(version)
    .fetch_all(&state.db)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(PoolTemplate {
                id: row.get("id"),
                probe_kind: row.get("probe_kind"),
                content: state
                    .crypto
                    .decrypt(&row.get::<String, _>("content_ciphertext"))
                    .map_err(|_| AppError::Internal("probe template decrypt failed".into()))?,
            })
        })
        .collect()
}

// ---------------------------------------------------------------- targets

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetInput {
    pub family: String,
    pub openrouter_model: String,
    pub provider_only: String,
    pub target_samples: i32,
    pub token_samples: i32,
    #[serde(default = "default_reasoning")]
    pub reasoning: String,
}

fn default_reasoning() -> String {
    "off".into()
}

pub async fn upsert_target(db: &PgPool, model: &str, input: &TargetInput) -> Result<()> {
    let clean = |value: &str, max: usize| {
        let value = value.trim();
        (!value.is_empty() && value.len() <= max && !value.chars().any(char::is_control))
            .then(|| value.to_owned())
    };
    let (Some(family), Some(openrouter_model), Some(provider_only)) = (
        clean(&input.family, 64),
        clean(&input.openrouter_model, 256),
        clean(&input.provider_only, 64),
    ) else {
        return Err(invalid("厂商、OpenRouter 模型 ID 和锁定供应商都需要填写"));
    };
    if !(20..=5000).contains(&input.target_samples) || !(0..=1000).contains(&input.token_samples) {
        return Err(invalid(
            "目标样本数需要在 20 到 5000 之间，Tokenizer 样本数在 0 到 1000 之间",
        ));
    }
    if !REASONING_MODES.contains(&input.reasoning.as_str()) {
        return Err(invalid("推理模式必须是 off 或 default"));
    }
    let known: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM model_catalog WHERE pattern=$1 AND strpos(pattern,'*')=0)",
    )
    .bind(model)
    .fetch_one(db)
    .await?;
    if !known {
        return Err(invalid("请先在模型管理中添加这个具体模型"));
    }
    sqlx::query(
        "INSERT INTO model_baseline_targets(model,family,openrouter_model,provider_only,target_samples,token_samples,reasoning)
         VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(model) DO UPDATE SET family=$2,openrouter_model=$3,
         provider_only=$4,target_samples=$5,token_samples=$6,reasoning=$7,updated_at=now()",
    )
    .bind(model)
    .bind(family)
    .bind(openrouter_model)
    .bind(provider_only)
    .bind(input.target_samples)
    .bind(input.token_samples)
    .bind(&input.reasoning)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn put_target(
    State(state): State<AppState>,
    Path(model): Path<String>,
    headers: HeaderMap,
    Json(input): Json<TargetInput>,
) -> Result<Json<Value>> {
    let admin = auth::require_super_admin(&state, &headers).await?;
    upsert_target(&state.db, &model, &input).await?;
    audit(
        &state.db,
        admin.account_id,
        admin.user_id,
        "platform.baseline_target_updated",
        "model_baseline_target",
        admin.user_id,
        json!({"model":model,"family":input.family,"openrouter_model":input.openrouter_model,
            "provider_only":input.provider_only,"target_samples":input.target_samples,
            "token_samples":input.token_samples,"reasoning":input.reasoning}),
    )
    .await;
    Ok(Json(json!({"model":model})))
}

pub async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    auth::require_super_admin(&state, &headers).await?;
    Ok(Json(overview(&state.db).await?))
}

/// Live batches of each version and its Tokenizer progress against the model's target,
/// counted per template the same way collection plans are.
async fn version_progress(
    db: &PgPool,
    versions: &[sqlx::postgres::PgRow],
) -> Result<HashMap<Uuid, Value>> {
    let ids: Vec<Uuid> = versions.iter().map(|v| v.get("id")).collect();
    let pools: Vec<i32> = versions.iter().map(|v| v.get("pool_version")).collect();
    let batches: HashMap<Uuid, i64> = sqlx::query_as(
        "SELECT version_id,count(*) FROM model_baseline_batches
         WHERE version_id=ANY($1) AND status='merged' AND excluded_at IS NULL GROUP BY 1",
    )
    .bind(&ids)
    .fetch_all(db)
    .await?
    .into_iter()
    .collect();
    let mut have: HashMap<Uuid, HashMap<Uuid, i64>> = HashMap::new();
    for (version, template, count) in sqlx::query_as::<_, (Uuid, Uuid, i64)>(
        "SELECT b.version_id,s.template_id,count(*) FROM model_baseline_samples s
         JOIN model_baseline_batches b ON b.id=s.batch_id
         WHERE b.version_id=ANY($1) AND b.status='merged' AND b.excluded_at IS NULL
           AND s.valid AND s.probe_kind='token' GROUP BY 1,2",
    )
    .bind(&ids)
    .fetch_all(db)
    .await?
    {
        have.entry(version).or_default().insert(template, count);
    }
    let mut templates: HashMap<i32, Vec<Uuid>> = HashMap::new();
    for (pool, template) in sqlx::query_as::<_, (i32, Uuid)>(
        "SELECT v.version,t.id FROM probe_pool_versions v CROSS JOIN LATERAL unnest(v.template_ids) AS u(id)
         JOIN probe_templates t ON t.id=u.id WHERE v.version=ANY($1) AND t.probe_kind='token' ORDER BY t.id",
    )
    .bind(&pools)
    .fetch_all(db)
    .await?
    {
        templates.entry(pool).or_default().push(template);
    }
    let none = HashMap::new();
    Ok(versions
        .iter()
        .map(|v| {
            let id: Uuid = v.get("id");
            let token_samples = v.get::<Option<i32>, _>("token_samples").unwrap_or(0);
            let templates = templates
                .get(&v.get::<i32, _>("pool_version"))
                .map(Vec::as_slice)
                .unwrap_or_default();
            let target: i64 = token_shortfall(token_samples, templates, &none)
                .values()
                .sum();
            let missing: i64 =
                token_shortfall(token_samples, templates, have.get(&id).unwrap_or(&none))
                    .values()
                    .sum();
            (
                id,
                json!({"batches":batches.get(&id).copied().unwrap_or(0),
                "token_target":target,"token_counted":target - missing,"token_missing":missing}),
            )
        })
        .collect())
}

pub async fn overview(db: &PgPool) -> Result<Value> {
    let targets = sqlx::query(
        "SELECT t.*,c.label FROM model_baseline_targets t LEFT JOIN model_catalog c ON c.pattern=t.model ORDER BY t.model",
    )
    .fetch_all(db)
    .await?;
    let versions = sqlx::query(
        "SELECT v.id,v.model,v.status,v.revision,v.valid_samples,v.pool_version,v.created_at,v.activated_at,
           e.results AS evaluation,e.revision AS evaluated_revision,e.created_at AS evaluated_at,t.token_samples
         FROM model_baseline_versions v
         LEFT JOIN model_baseline_targets t ON t.model=v.model
         LEFT JOIN LATERAL (SELECT results,revision,created_at FROM model_baseline_evaluations
             WHERE version_id=v.id ORDER BY created_at DESC LIMIT 1) e ON true
         WHERE v.status IN ('draft','active') ORDER BY v.created_at DESC",
    )
    .fetch_all(db)
    .await?;
    let pending: HashMap<String, i64> = sqlx::query_as(
        "SELECT model,count(*) FROM model_baseline_batches WHERE status='pending' GROUP BY model",
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .collect();
    let pool_version: Option<i32> =
        sqlx::query_scalar("SELECT max(version) FROM probe_pool_versions")
            .fetch_one(db)
            .await?;
    let progress = version_progress(db, &versions).await?;
    let version_json = |row: &sqlx::postgres::PgRow| {
        json!({"id":row.get::<Uuid,_>("id"),"status":row.get::<String,_>("status"),
            "progress":progress.get(&row.get::<Uuid,_>("id")),
            "revision":row.get::<i32,_>("revision"),"valid_samples":row.get::<i32,_>("valid_samples"),
            "pool_version":row.get::<i32,_>("pool_version"),"created_at":row.get::<DateTime<Utc>,_>("created_at"),
            "activated_at":row.get::<Option<DateTime<Utc>>,_>("activated_at"),
            "evaluation":row.get::<Option<Value>,_>("evaluation"),
            "evaluated_revision":row.get::<Option<i32>,_>("evaluated_revision"),
            "evaluated_at":row.get::<Option<DateTime<Utc>>,_>("evaluated_at")})
    };
    let models: Vec<Value> = targets
        .iter()
        .map(|t| {
            let model: String = t.get("model");
            let active = versions
                .iter()
                .find(|v| v.get::<String, _>("model") == model && v.get::<String, _>("status") == "active")
                .map(version_json);
            let draft = versions
                .iter()
                .find(|v| v.get::<String, _>("model") == model && v.get::<String, _>("status") == "draft")
                .map(version_json);
            json!({"model":model,"label":t.get::<Option<String>,_>("label"),
                "family":t.get::<String,_>("family"),"openrouter_model":t.get::<String,_>("openrouter_model"),
                "provider_only":t.get::<String,_>("provider_only"),"target_samples":t.get::<i32,_>("target_samples"),
                "token_samples":t.get::<i32,_>("token_samples"),"reasoning":t.get::<String,_>("reasoning"),"active":active,"draft":draft,
                "pending_batches":pending.get(&model).copied().unwrap_or(0)})
        })
        .collect();
    Ok(json!({"models":models,"pool_version":pool_version,"range_max":RANGE_MAX}))
}

// ---------------------------------------------------------------- OpenRouter lookup

#[derive(Deserialize)]
pub struct LookupQuery {
    model: String,
    openrouter_model: Option<String>,
}

async fn openrouter_get(path: &str) -> std::result::Result<Value, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?
        .get(format!("https://openrouter.ai/api/v1{path}"))
        .header("User-Agent", "meta-router/1.0")
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())
        .map(|response| {
            crate::gateway::read_json_limited(response, crate::gateway::MAX_METADATA_BODY)
        })?
        .await
}

/// OpenRouter ids whose native name is `model`, using the price sync's
/// conversion ("anthropic/claude-sonnet-5.5" -> "claude-sonnet-5-5").
fn matching_ids(models: &Value, model: &str) -> Vec<Value> {
    models["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let id = m["id"].as_str()?;
            (!id.contains(':') && pricing::native_model_name(id).as_deref() == Some(model)).then(
                || {
                    let reasoning = m["supported_parameters"]
                        .as_array()
                        .is_some_and(|p| p.iter().any(|v| v == "reasoning"));
                    json!({"id":id,"name":m["name"],"supports_reasoning":reasoning})
                },
            )
        })
        .collect()
}

/// Providers serving an OpenRouter model; `slug` is what `provider.only` takes.
fn providers(endpoints: &Value) -> Vec<Value> {
    let mut seen = std::collections::HashSet::new();
    endpoints["data"]["endpoints"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let slug = e["tag"].as_str()?.split('/').next()?.to_owned();
            seen.insert(slug.clone()).then(
                || json!({"slug":slug,"name":e["provider_name"],"quantization":e["quantization"]}),
            )
        })
        .collect()
}

/// Suggests the collection settings for a price-table model from OpenRouter.
pub async fn openrouter_lookup(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<LookupQuery>,
) -> Result<Json<Value>> {
    auth::require_super_admin(&state, &headers).await?;
    let provider: Option<String> = sqlx::query_scalar(
        "SELECT NULLIF(btrim(provider),'') FROM model_catalog WHERE pattern=$1 AND strpos(pattern,'*')=0",
    )
    .bind(&query.model)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| invalid("价格表中没有这个具体模型"))?;
    let unavailable =
        |_| AppError::ServiceUnavailable("暂时无法访问 OpenRouter，请稍后重试或手动填写".into());
    let models = openrouter_get("/models").await.map_err(unavailable)?;
    let candidates = matching_ids(&models, &query.model);
    // Prefer the id whose vendor prefix matches the price table's vendor.
    let chosen = query.openrouter_model.clone().or_else(|| {
        candidates
            .iter()
            .filter_map(|c| c["id"].as_str())
            .find(|id| {
                provider
                    .as_deref()
                    .is_some_and(|p| id.starts_with(&format!("{p}/")))
            })
            .or_else(|| candidates.first().and_then(|c| c["id"].as_str()))
            .map(str::to_owned)
    });
    let Some(chosen) = chosen else {
        return Ok(Json(
            json!({"candidates":candidates,"openrouter_model":null,"providers":[],
            "family":provider,"provider_only":null}),
        ));
    };
    let vendor = chosen.split('/').next().unwrap_or_default().to_owned();
    let endpoints = openrouter_get(&format!("/models/{chosen}/endpoints"))
        .await
        .map_err(unavailable)?;
    let providers = providers(&endpoints);
    // The official provider usually shares the vendor's slug (anthropic, openai);
    // a few vendors serve under another name.
    let official_slug = match vendor.as_str() {
        "qwen" => "alibaba",
        "google" => "google-ai-studio",
        "mistralai" => "mistral",
        "x-ai" => "xai",
        other => other,
    };
    let official = providers
        .iter()
        .filter_map(|p| p["slug"].as_str())
        .find(|slug| *slug == official_slug)
        .map(str::to_owned);
    Ok(Json(
        json!({"candidates":candidates,"openrouter_model":chosen,"providers":providers,
        "family":provider.unwrap_or(vendor),"provider_only":official}),
    ))
}

// ---------------------------------------------------------------- versions

/// The newest draft or active version collected under these conditions.
async fn compatible_version(
    db: &mut PgConnection,
    model: &str,
    provider_only: &str,
    pool_version: i32,
    reasoning: &str,
) -> Result<Option<Uuid>> {
    Ok(sqlx::query_scalar(
        "SELECT id FROM model_baseline_versions WHERE model=$1 AND status IN ('draft','active')
         AND source=$2 AND provider_only=$3 AND pool_version=$4 AND reasoning=$5 AND range_max=$6
         AND format_version=$7 ORDER BY (status='active') DESC,created_at DESC LIMIT 1",
    )
    .bind(model)
    .bind(SOURCE)
    .bind(provider_only)
    .bind(pool_version)
    .bind(reasoning)
    .bind(i32::from(RANGE_MAX))
    .bind(bundle::FORMAT_VERSION)
    .fetch_optional(&mut *db)
    .await?)
}

/// Valid fingerprint samples of a version's included batches.
pub async fn version_samples(db: &mut PgConnection, version_id: Uuid) -> Result<Vec<Vec<u16>>> {
    let rows: Vec<Vec<i32>> = sqlx::query_scalar(
        "SELECT s.sample_values FROM model_baseline_samples s JOIN model_baseline_batches b ON b.id=s.batch_id
         WHERE b.version_id=$1 AND b.status='merged' AND b.excluded_at IS NULL
           AND s.valid AND s.probe_kind='fingerprint' ORDER BY b.created_at,s.sample_id",
    )
    .bind(version_id)
    .fetch_all(&mut *db)
    .await?;
    Ok(rows.into_iter().map(to_sample).collect())
}

fn to_sample(values: Vec<i32>) -> Vec<u16> {
    values
        .into_iter()
        .filter_map(|v| u16::try_from(v).ok())
        .collect()
}

fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let mid = values.len() / 2;
    Some(if values.len().is_multiple_of(2) {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    })
}

/// Recomputes a version's statistics from its included batches and bumps the revision.
async fn recompute(db: &mut PgConnection, version_id: Uuid) -> Result<()> {
    let samples = version_samples(&mut *db, version_id).await?;
    let usage: (Option<f64>, Option<f64>) = sqlx::query_as(
        "SELECT avg(s.input_tokens)::float8,avg(s.output_tokens)::float8 FROM model_baseline_samples s
         JOIN model_baseline_batches b ON b.id=s.batch_id WHERE b.version_id=$1 AND b.status='merged'
         AND b.excluded_at IS NULL AND s.valid AND s.probe_kind='fingerprint'",
    )
    .bind(version_id)
    .fetch_one(&mut *db)
    .await?;
    let tokens = sqlx::query(
        "SELECT s.template_id,s.input_tokens FROM model_baseline_samples s JOIN model_baseline_batches b ON b.id=s.batch_id
         WHERE b.version_id=$1 AND b.status='merged' AND b.excluded_at IS NULL AND s.valid AND s.probe_kind='token'",
    )
    .bind(version_id)
    .fetch_all(&mut *db)
    .await?;
    let mut by_template: BTreeMap<Uuid, Vec<f64>> = BTreeMap::new();
    for row in tokens {
        if let Some(count) = row.get::<Option<i32>, _>("input_tokens") {
            by_template
                .entry(row.get("template_id"))
                .or_default()
                .push(f64::from(count));
        }
    }
    let stats = Stats {
        fingerprint: (!samples.is_empty())
            .then(|| Counts::from_samples(RANGE_MAX, samples.iter().map(Vec::as_slice))),
        token: by_template
            .into_iter()
            .filter_map(|(id, mut counts)| median(&mut counts).map(|m| (id, m)))
            .collect(),
        avg_input: usage.0.unwrap_or(0.0),
        avg_output: usage.1.unwrap_or(0.0),
    };
    sqlx::query(
        "UPDATE model_baseline_versions SET stats=$2,valid_samples=$3,revision=revision+1 WHERE id=$1",
    )
    .bind(version_id)
    .bind(json!(stats))
    .bind(samples.len() as i32)
    .execute(&mut *db)
    .await?;
    Ok(())
}

pub struct ActiveBaseline {
    pub version_id: Uuid,
    pub revision: i32,
    pub model: String,
    pub family: String,
    pub pool_version: i32,
    pub reasoning: String,
    pub stats: Stats,
}

pub async fn active_baselines(db: &PgPool) -> Result<Vec<ActiveBaseline>> {
    let rows = sqlx::query(
        "SELECT id,revision,model,family,pool_version,reasoning,stats FROM model_baseline_versions WHERE status='active' ORDER BY model",
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let stats: Stats = serde_json::from_value(row.get("stats")).ok()?;
            Some(ActiveBaseline {
                version_id: row.get("id"),
                revision: row.get("revision"),
                model: row.get("model"),
                family: row.get("family"),
                pool_version: row.get("pool_version"),
                reasoning: row.get("reasoning"),
                stats,
            })
        })
        .collect())
}

/// Evaluates a version against every other model's active version and stores the result.
pub async fn evaluate_version(db: &PgPool, version_id: Uuid) -> Result<Option<Value>> {
    let mut conn = db.acquire().await?;
    let row = sqlx::query("SELECT model,family,revision FROM model_baseline_versions WHERE id=$1")
        .bind(version_id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(AppError::NotFound)?;
    let model: String = row.get("model");
    let revision: i32 = row.get("revision");
    let mut data = vec![ModelData {
        model: model.clone(),
        family: row.get("family"),
        samples: version_samples(&mut conn, version_id).await?,
    }];
    let others = sqlx::query(
        "SELECT id,model,family,revision FROM model_baseline_versions WHERE status='active' AND model<>$1",
    )
    .bind(&model)
    .fetch_all(&mut *conn)
    .await?;
    let mut candidates = Vec::new();
    for other in others {
        let id: Uuid = other.get("id");
        candidates.push(
            json!({"model":other.get::<String,_>("model"),"version_id":id,
            "revision":other.get::<i32,_>("revision")}),
        );
        data.push(ModelData {
            model: other.get("model"),
            family: other.get("family"),
            samples: version_samples(&mut conn, id).await?,
        });
    }
    drop(conn);
    let seed = version_id.as_u128() as u64 ^ revision as u64;
    let target = model.clone();
    let result = tokio::task::spawn_blocking(move || {
        evaluate::evaluate(&target, &data, RANGE_MAX, 30, 100, seed)
    })
    .await
    .map_err(|_| AppError::Internal("evaluation task failed".into()))?;
    let Some(result) = result else {
        return Ok(None);
    };
    let results = json!(result);
    sqlx::query(
        "INSERT INTO model_baseline_evaluations(version_id,revision,candidates,results) VALUES($1,$2,$3,$4)",
    )
    .bind(version_id)
    .bind(revision)
    .bind(json!(candidates))
    .bind(&results)
    .execute(db)
    .await?;
    Ok(Some(results))
}

/// Re-evaluates a changed version and every active version: each evaluation
/// compares against the other active baselines, so one change affects all.
pub async fn evaluate_affected(db: &PgPool, version_id: Uuid) {
    let mut ids = vec![version_id];
    match sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM model_baseline_versions WHERE status='active' AND id<>$1 ORDER BY model",
    )
    .bind(version_id)
    .fetch_all(db)
    .await
    {
        Ok(active) => ids.extend(active),
        Err(error) => eprintln!("baseline evaluation listing failed: {error}"),
    }
    for id in ids {
        if let Err(error) = evaluate_version(db, id).await {
            eprintln!("baseline evaluation {id} failed: {error}");
        }
    }
}

fn spawn_evaluation(db: &PgPool, version_id: Uuid) {
    let db = db.clone();
    tokio::spawn(async move { evaluate_affected(&db, version_id).await });
}

pub async fn list_versions(
    State(state): State<AppState>,
    Path(model): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    auth::require_super_admin(&state, &headers).await?;
    let versions = sqlx::query(
        "SELECT v.id,v.status,v.revision,v.valid_samples,v.pool_version,v.provider_only,v.created_at,v.activated_at,
           e.results AS evaluation,e.revision AS evaluated_revision,e.created_at AS evaluated_at,t.token_samples
         FROM model_baseline_versions v
         LEFT JOIN model_baseline_targets t ON t.model=v.model
         LEFT JOIN LATERAL (SELECT results,revision,created_at FROM model_baseline_evaluations
             WHERE version_id=v.id ORDER BY created_at DESC LIMIT 1) e ON true
         WHERE v.model=$1 ORDER BY v.created_at DESC",
    )
    .bind(&model)
    .fetch_all(&state.db)
    .await?;
    let batches = sqlx::query(
        "SELECT id,version_id,status,valid_count,sample_count,created_at,excluded_at FROM model_baseline_batches
         WHERE model=$1 AND status='merged' ORDER BY created_at",
    )
    .bind(&model)
    .fetch_all(&state.db)
    .await?;
    let progress = version_progress(&state.db, &versions).await?;
    let ids: Vec<Uuid> = batches.iter().map(|b| b.get("id")).collect();
    let counts = sample_counts(&state.db, &ids).await?;
    let out: Vec<Value> = versions
        .iter()
        .map(|v| {
            let id: Uuid = v.get("id");
            let own: Vec<Value> = batches
                .iter()
                .filter(|b| b.get::<Option<Uuid>, _>("version_id") == Some(id))
                .map(|b| json!({"id":b.get::<Uuid,_>("id"),"valid_count":b.get::<i32,_>("valid_count"),
                    "counts":counts.get(&b.get::<Uuid,_>("id")).cloned().unwrap_or_else(|| json!({})),
                    "sample_count":b.get::<i32,_>("sample_count"),"created_at":b.get::<DateTime<Utc>,_>("created_at"),
                    "excluded_at":b.get::<Option<DateTime<Utc>>,_>("excluded_at")}))
                .collect();
            json!({"id":id,"status":v.get::<String,_>("status"),"revision":v.get::<i32,_>("revision"),
                "valid_samples":v.get::<i32,_>("valid_samples"),"pool_version":v.get::<i32,_>("pool_version"),
                "provider_only":v.get::<String,_>("provider_only"),"created_at":v.get::<DateTime<Utc>,_>("created_at"),
                "activated_at":v.get::<Option<DateTime<Utc>>,_>("activated_at"),
                "evaluation":v.get::<Option<Value>,_>("evaluation"),
                "evaluated_revision":v.get::<Option<i32>,_>("evaluated_revision"),
                "evaluated_at":v.get::<Option<DateTime<Utc>>,_>("evaluated_at"),"batches":own,
                "progress":progress.get(&id)})
        })
        .collect();
    Ok(Json(json!(out)))
}

pub async fn activate_version(db: &PgPool, version_id: Uuid) -> Result<String> {
    let mut tx = db.begin().await?;
    let row = sqlx::query(
        "SELECT model,status,valid_samples FROM model_baseline_versions WHERE id=$1 FOR UPDATE",
    )
    .bind(version_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    let model: String = row.get("model");
    if row.get::<String, _>("status") == "active" {
        return Ok(model);
    }
    if (row.get::<i32, _>("valid_samples") as usize) < MIN_EVAL_SAMPLES {
        return Err(AppError::Conflict(format!(
            "至少需要 {MIN_EVAL_SAMPLES} 个有效指纹样本才能启用"
        )));
    }
    sqlx::query(
        "UPDATE model_baseline_versions SET status='retired' WHERE model=$1 AND status='active'",
    )
    .bind(&model)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE model_baseline_versions SET status='active',activated_at=now() WHERE id=$1",
    )
    .bind(version_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(model)
}

pub async fn activate(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let admin = auth::require_super_admin(&state, &headers).await?;
    let model = activate_version(&state.db, id).await?;
    spawn_evaluation(&state.db, id);
    audit(
        &state.db,
        admin.account_id,
        admin.user_id,
        "platform.baseline_version_activated",
        "model_baseline_version",
        id,
        json!({"model":model}),
    )
    .await;
    Ok(Json(json!({"id":id})))
}

// ---------------------------------------------------------------- plans

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanInput {
    pub models: Vec<String>,
}

/// Tokenizer samples are collected evenly per template; returns what each template still lacks.
fn token_shortfall(
    token_samples: i32,
    templates: &[Uuid],
    have: &HashMap<Uuid, i64>,
) -> BTreeMap<Uuid, i64> {
    let per_template = if templates.is_empty() {
        0
    } else {
        (i64::from(token_samples) + templates.len() as i64 - 1) / templates.len() as i64
    };
    templates
        .iter()
        .map(|id| {
            (
                *id,
                (per_template - have.get(id).copied().unwrap_or(0)).max(0),
            )
        })
        .collect()
}

pub async fn create_plan_for(db: &PgPool, user_id: Uuid, models: &[String]) -> Result<Value> {
    if models.is_empty() || models.len() > 50 {
        return Err(invalid("请选择 1 到 50 个模型"));
    }
    let mut conn = db.acquire().await?;
    let pool_version: i32 = sqlx::query_scalar("SELECT max(version) FROM probe_pool_versions")
        .fetch_one(&mut *conn)
        .await
        .ok()
        .flatten()
        .ok_or_else(|| AppError::Conflict("请先在提示词池中发布一个版本".into()))?;
    let token_templates: Vec<Uuid> = sqlx::query_scalar(
        "SELECT t.id FROM probe_pool_versions v CROSS JOIN LATERAL unnest(v.template_ids) AS u(id)
         JOIN probe_templates t ON t.id=u.id WHERE v.version=$1 AND t.probe_kind='token' ORDER BY t.id",
    )
    .bind(pool_version)
    .fetch_all(&mut *conn)
    .await?;
    let prices = pricing::load(db).await?;
    let mut items = Vec::new();
    let mut estimate = 0.0;
    for model in models {
        let target = sqlx::query("SELECT * FROM model_baseline_targets WHERE model=$1")
            .bind(model)
            .fetch_optional(&mut *conn)
            .await?
            .ok_or_else(|| invalid(&format!("{model} 还没有基准配置")))?;
        let provider_only: String = target.get("provider_only");
        let reasoning: String = target.get("reasoning");
        let version =
            compatible_version(&mut conn, model, &provider_only, pool_version, &reasoning).await?;
        let (fingerprint_have, token_have): (i64, HashMap<Uuid, i64>) = match version {
            Some(id) => {
                let fp: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM model_baseline_samples s JOIN model_baseline_batches b ON b.id=s.batch_id
                     WHERE b.version_id=$1 AND b.status='merged' AND b.excluded_at IS NULL AND s.valid AND s.probe_kind='fingerprint'",
                )
                .bind(id)
                .fetch_one(&mut *conn)
                .await?;
                let tk: Vec<(Uuid, i64)> = sqlx::query_as(
                    "SELECT s.template_id,count(*) FROM model_baseline_samples s JOIN model_baseline_batches b ON b.id=s.batch_id
                     WHERE b.version_id=$1 AND b.status='merged' AND b.excluded_at IS NULL AND s.valid AND s.probe_kind='token'
                     GROUP BY s.template_id",
                )
                .bind(id)
                .fetch_all(&mut *conn)
                .await?;
                (fp, tk.into_iter().collect())
            }
            None => (0, HashMap::new()),
        };
        let fingerprint =
            (i64::from(target.get::<i32, _>("target_samples")) - fingerprint_have).max(0);
        let token = token_shortfall(
            target.get::<i32, _>("token_samples"),
            &token_templates,
            &token_have,
        );
        if let Some(price) = pricing::lookup(&prices, model) {
            let token_total: i64 = token.values().sum();
            estimate += fingerprint as f64
                * price.estimate(FINGERPRINT_TOKENS.0, FINGERPRINT_TOKENS.1, 0)
                + token_total as f64 * price.estimate(TOKEN_TOKENS.0, TOKEN_TOKENS.1, 0);
        }
        items.push(json!({"model":model,"family":target.get::<String,_>("family"),
            "openrouter_model":target.get::<String,_>("openrouter_model"),"provider_only":provider_only,
            "reasoning":reasoning,"version_id":version,"fingerprint":fingerprint,"token":token}));
    }
    let expires_at = Utc::now() + Duration::days(PLAN_DAYS);
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO model_baseline_plans(pool_version,items,created_by,expires_at) VALUES($1,$2,$3,$4) RETURNING id",
    )
    .bind(pool_version)
    .bind(json!(items))
    .bind(user_id)
    .bind(expires_at)
    .fetch_one(&mut *conn)
    .await?;
    Ok(
        json!({"id":id,"pool_version":pool_version,"items":items,"expires_at":expires_at,
        "estimated_cost_usd":estimate}),
    )
}

pub async fn create_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<PlanInput>,
) -> Result<(StatusCode, Json<Value>)> {
    let admin = auth::require_super_admin(&state, &headers).await?;
    let plan = create_plan_for(&state.db, admin.user_id, &input.models).await?;
    let id: Uuid = serde_json::from_value(plan["id"].clone()).unwrap_or_default();
    audit(
        &state.db,
        admin.account_id,
        admin.user_id,
        "platform.baseline_plan_created",
        "model_baseline_plan",
        id,
        json!({"models":input.models,"pool_version":plan["pool_version"]}),
    )
    .await;
    Ok((StatusCode::CREATED, Json(plan)))
}

/// The plan file for the collector. It contains the secret prompt pool.
pub async fn plan_file(state: &AppState, id: Uuid) -> Result<Value> {
    let row = sqlx::query(
        "UPDATE model_baseline_plans SET downloaded_at=now() WHERE id=$1 AND expires_at>now()
         RETURNING pool_version,items,expires_at",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::Conflict("采集计划不存在或已过期，请重新生成".into()))?;
    let pool_version: i32 = row.get("pool_version");
    let templates: Vec<Value> = pool_templates(state, pool_version)
        .await?
        .into_iter()
        .map(|t| json!({"id":t.id,"probe_kind":t.probe_kind,"content":t.content}))
        .collect();
    Ok(
        json!({"format":"vair-baseline-plan","format_version":2,"plan_id":id,"pool_version":pool_version,
        "range_max":RANGE_MAX,"source":SOURCE,"bundle_format_version":bundle::FORMAT_VERSION,
        "expires_at":row.get::<DateTime<Utc>,_>("expires_at"),"templates":templates,
        "items":row.get::<Value,_>("items")}),
    )
}

pub async fn download_plan(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let admin = auth::require_super_admin(&state, &headers).await?;
    let file = plan_file(&state, id).await?;
    audit(
        &state.db,
        admin.account_id,
        admin.user_id,
        "platform.baseline_plan_downloaded",
        "model_baseline_plan",
        id,
        json!({"pool_version":file["pool_version"]}),
    )
    .await;
    Ok(Json(file))
}

// ---------------------------------------------------------------- imports

struct SampleRow {
    sample_id: String,
    probe_kind: String,
    template_id: Uuid,
    n: Option<i32>,
    values: Vec<i32>,
    input_tokens: Option<i32>,
    output_tokens: Option<i32>,
    reasoning_tokens: Option<i32>,
    served_provider: Option<String>,
    valid: bool,
    invalid_reason: Option<&'static str>,
}

fn clamp_i32(value: Option<i64>) -> Option<i32> {
    value.map(|v| v.clamp(0, i64::from(i32::MAX)) as i32)
}

/// Upstreams that omit the serving provider are reported apart from ones that name another.
fn provider_reason(provider_only: &str, served: Option<&str>) -> Option<&'static str> {
    match served {
        None => Some("no_provider"),
        Some(p) if !bundle::same_provider(provider_only, p) => Some("provider_mismatch"),
        Some(_) => None,
    }
}

/// Re-validates every sample with the server's rules; the bundle's own view is not trusted.
fn check_samples(
    bundle: &bundle::Bundle,
    pool: &HashMap<Uuid, String>,
) -> (Vec<SampleRow>, BTreeMap<&'static str, usize>) {
    let reasoning_off = bundle.manifest.reasoning == "off";
    let mut reasons = BTreeMap::new();
    let rows = bundle
        .samples
        .iter()
        .map(|s| {
            let mut values = Vec::new();
            let reason = if pool.get(&s.template_id) != Some(&s.probe_kind) {
                Some("unknown_template")
            } else if let Some(reason) =
                provider_reason(&bundle.manifest.provider_only, s.served_provider.as_deref())
            {
                Some(reason)
            } else if reasoning_off && s.reasoning_tokens.unwrap_or(0) > MAX_REASONING_TOKENS {
                Some("reasoning")
            } else if s.probe_kind == "token" {
                (s.input_tokens.unwrap_or(0) <= 0).then_some("no_usage")
            } else {
                match s.n.filter(|n| (1..=1000).contains(n)) {
                    None => Some("bad_n"),
                    Some(n) => match fingerprint::validate(&s.values, n as usize, RANGE_MAX) {
                        Ok(clean) => {
                            values = clean.into_iter().map(i32::from).collect();
                            None
                        }
                        Err(reason) => Some(reason),
                    },
                }
            };
            if let Some(reason) = reason {
                *reasons.entry(reason).or_default() += 1;
            }
            SampleRow {
                sample_id: s.sample_id.clone(),
                probe_kind: s.probe_kind.clone(),
                template_id: s.template_id,
                n: s.n,
                values,
                input_tokens: clamp_i32(s.input_tokens),
                output_tokens: clamp_i32(s.output_tokens),
                reasoning_tokens: clamp_i32(s.reasoning_tokens),
                served_provider: s
                    .served_provider
                    .as_deref()
                    .map(|p| p.chars().take(64).collect()),
                valid: reason.is_none(),
                invalid_reason: reason,
            }
        })
        .collect();
    (rows, reasons)
}

pub async fn import_bundle(db: &PgPool, user_id: Uuid, bytes: &[u8]) -> Result<(bool, Value)> {
    let bundle = bundle::parse(bytes).map_err(AppError::BadRequest)?;
    let m = &bundle.manifest;
    // Re-uploading the same bundle returns the existing batch.
    if let Some(id) = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM model_baseline_batches WHERE id=$1 OR samples_sha256=$2",
    )
    .bind(m.batch_id)
    .bind(&bundle.sha256)
    .fetch_optional(db)
    .await?
    {
        return Ok((false, batch_view(db, id).await?));
    }
    let plan = sqlx::query(
        "SELECT pool_version,items,expires_at>now() AS live FROM model_baseline_plans WHERE id=$1",
    )
    .bind(m.plan_id)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| invalid("导入包引用的采集计划不存在"))?;
    if !plan.get::<bool, _>("live") {
        return Err(invalid("采集计划已过期，请重新生成计划后再采集"));
    }
    let items: Value = plan.get("items");
    if !items
        .as_array()
        .is_some_and(|items| items.iter().any(|i| i["model"] == m.model.as_str()))
    {
        return Err(invalid("采集计划中没有这个模型"));
    }
    let target = sqlx::query(
        "SELECT provider_only,openrouter_model,reasoning FROM model_baseline_targets WHERE model=$1",
    )
    .bind(&m.model)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| invalid("这个模型没有基准配置"))?;
    let mut mismatches = Vec::new();
    if m.pool_version != plan.get::<i32, _>("pool_version") {
        mismatches.push("提示词池版本");
    }
    if m.source != SOURCE {
        mismatches.push("基准来源");
    }
    if m.reasoning != target.get::<String, _>("reasoning") {
        mismatches.push("推理设置");
    }
    if m.range_max != i32::from(RANGE_MAX) {
        mismatches.push("取值范围");
    }
    if m.provider_only != target.get::<String, _>("provider_only") {
        mismatches.push("锁定供应商");
    }
    if !mismatches.is_empty() {
        return Err(invalid(&format!(
            "采集条件与计划或基准配置不一致：{}",
            mismatches.join("、")
        )));
    }
    let pool: HashMap<Uuid, String> = sqlx::query_as(
        "SELECT t.id,t.probe_kind FROM probe_pool_versions v CROSS JOIN LATERAL unnest(v.template_ids) AS u(id)
         JOIN probe_templates t ON t.id=u.id WHERE v.version=$1",
    )
    .bind(m.pool_version)
    .fetch_all(db)
    .await?
    .into_iter()
    .collect();
    let (rows, reasons) = check_samples(&bundle, &pool);
    let valid_count = rows.iter().filter(|r| r.valid).count();
    let batch_samples: Vec<Vec<u16>> = rows
        .iter()
        .filter(|r| r.valid && r.probe_kind == "fingerprint")
        .map(|r| to_sample(r.values.clone()))
        .collect();
    let mut conn = db.acquire().await?;
    let compatible = compatible_version(
        &mut conn,
        &m.model,
        &m.provider_only,
        m.pool_version,
        &m.reasoning,
    )
    .await?;
    let existing = match compatible {
        Some(id) => version_samples(&mut conn, id).await?,
        None => Vec::new(),
    };
    drop(conn);
    let validation = json!({"samples":bundle.samples.len(),"duplicates":bundle.duplicates,
        "valid":valid_count,"fingerprint_valid":batch_samples.len(),
        "token_valid":rows.iter().filter(|r| r.valid && r.probe_kind=="token").count(),
        "invalid":reasons,"compatible_version_id":compatible});
    let seed = m.batch_id.as_u128() as u64;
    let (drift, merged) = {
        let existing = existing.clone();
        let batch = batch_samples.clone();
        tokio::task::spawn_blocking(move || {
            let drift = evaluate::drift(&existing, &batch, RANGE_MAX, 200, seed);
            let mut merged = existing;
            merged.extend(batch);
            (drift, merged)
        })
        .await
        .map_err(|_| AppError::Internal("drift task failed".into()))?
    };
    let preview = preview_evaluation(db, &m.model, merged, seed).await?;
    let mut tx = db.begin().await?;
    sqlx::query(
        "INSERT INTO model_baseline_batches(id,model,plan_id,manifest,samples_sha256,validation,drift,preview,
         sample_count,valid_count,imported_by) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
    )
    .bind(m.batch_id)
    .bind(&m.model)
    .bind(m.plan_id)
    .bind(json!(m))
    .bind(&bundle.sha256)
    .bind(&validation)
    .bind(json!(drift))
    .bind(&preview)
    .bind(bundle.samples.len() as i32)
    .bind(valid_count as i32)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    for chunk in rows.chunks(500) {
        let mut query = sqlx::QueryBuilder::new(
            "INSERT INTO model_baseline_samples(batch_id,sample_id,probe_kind,template_id,n,sample_values,
             input_tokens,output_tokens,reasoning_tokens,served_provider,valid,invalid_reason) ",
        );
        query.push_values(chunk, |mut b, r| {
            b.push_bind(m.batch_id)
                .push_bind(&r.sample_id)
                .push_bind(&r.probe_kind)
                .push_bind(r.template_id)
                .push_bind(r.n)
                .push_bind(&r.values)
                .push_bind(r.input_tokens)
                .push_bind(r.output_tokens)
                .push_bind(r.reasoning_tokens)
                .push_bind(&r.served_provider)
                .push_bind(r.valid)
                .push_bind(r.invalid_reason);
        });
        query.build().execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok((true, batch_view(db, m.batch_id).await?))
}

/// Evaluates the would-be merged samples with fewer splits than a stored evaluation.
async fn preview_evaluation(
    db: &PgPool,
    model: &str,
    merged: Vec<Vec<u16>>,
    seed: u64,
) -> Result<Value> {
    let family: Option<String> =
        sqlx::query_scalar("SELECT family FROM model_baseline_targets WHERE model=$1")
            .bind(model)
            .fetch_optional(db)
            .await?;
    let mut data = vec![ModelData {
        model: model.to_owned(),
        family: family.unwrap_or_default(),
        samples: merged,
    }];
    let mut conn = db.acquire().await?;
    let others = sqlx::query(
        "SELECT id,model,family FROM model_baseline_versions WHERE status='active' AND model<>$1",
    )
    .bind(model)
    .fetch_all(&mut *conn)
    .await?;
    for other in others {
        data.push(ModelData {
            model: other.get("model"),
            family: other.get("family"),
            samples: version_samples(&mut conn, other.get("id")).await?,
        });
    }
    drop(conn);
    let target = model.to_owned();
    let result = tokio::task::spawn_blocking(move || {
        evaluate::evaluate(&target, &data, RANGE_MAX, 10, 60, seed)
    })
    .await
    .map_err(|_| AppError::Internal("evaluation task failed".into()))?;
    Ok(json!(result))
}

pub async fn import(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>)> {
    let admin = auth::require_super_admin(&state, &headers).await?;
    let (created, view) = import_bundle(&state.db, admin.user_id, &body).await?;
    if created {
        audit(&state.db, admin.account_id, admin.user_id, "platform.baseline_imported",
            "model_baseline_batch", serde_json::from_value(view["id"].clone()).unwrap_or_default(),
            json!({"model":view["model"],"valid_count":view["valid_count"],"sample_count":view["sample_count"],
                "samples_sha256":view["samples_sha256"]})).await;
    }
    Ok((
        if created {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(view),
    ))
}

/// A batch with the version it joined, the version it could join and the model's sample target.
const BATCH_SELECT: &str = "SELECT b.*,v.revision AS version_revision,v.status AS version_status,
       v.valid_samples AS version_valid_samples,cv.revision AS compatible_revision,t.target_samples
     FROM model_baseline_batches b
     LEFT JOIN model_baseline_versions v ON v.id=b.version_id
     LEFT JOIN model_baseline_versions cv ON cv.id=(b.validation->>'compatible_version_id')::uuid
     LEFT JOIN model_baseline_targets t ON t.model=b.model";

/// Per-kind sample counts of each batch, read from the stored samples. Older imports recorded
/// a missing serving provider as `provider_mismatch`; the stored provider tells them apart.
async fn sample_counts(db: &PgPool, ids: &[Uuid]) -> Result<HashMap<Uuid, Value>> {
    let rows: Vec<(Uuid, String, bool, Option<String>, i64)> = sqlx::query_as(
        "SELECT batch_id,probe_kind,valid,
           CASE WHEN invalid_reason='provider_mismatch' AND served_provider IS NULL THEN 'no_provider'
                ELSE invalid_reason END,count(*)
         FROM model_baseline_samples WHERE batch_id=ANY($1) GROUP BY 1,2,3,4",
    )
    .bind(ids)
    .fetch_all(db)
    .await?;
    let mut out: HashMap<Uuid, Value> = HashMap::new();
    for (batch, kind, valid, reason, count) in rows {
        let entry = &mut out.entry(batch).or_insert_with(|| json!({}))[kind.as_str()];
        if entry.is_null() {
            *entry = json!({"total":0,"valid":0,"invalid":{}});
        }
        entry["total"] = json!(entry["total"].as_i64().unwrap_or(0) + count);
        if valid {
            entry["valid"] = json!(entry["valid"].as_i64().unwrap_or(0) + count);
        } else {
            let reason = reason.unwrap_or_else(|| "unknown".into());
            let slot = &mut entry["invalid"][reason.as_str()];
            *slot = json!(slot.as_i64().unwrap_or(0) + count);
        }
    }
    Ok(out)
}

async fn batch_view(db: &PgPool, id: Uuid) -> Result<Value> {
    let r = sqlx::query(&format!("{BATCH_SELECT} WHERE b.id=$1"))
        .bind(id)
        .fetch_optional(db)
        .await?
        .ok_or(AppError::NotFound)?;
    let counts = sample_counts(db, &[id]).await?;
    Ok(batch_json(&r, &counts))
}

fn batch_json(r: &sqlx::postgres::PgRow, counts: &HashMap<Uuid, Value>) -> Value {
    let id: Uuid = r.get("id");
    let version = r.get::<Option<i32>, _>("version_revision").map(|revision| {
        json!({"revision":revision,"status":r.get::<Option<String>,_>("version_status"),
            "valid_samples":r.get::<Option<i32>,_>("version_valid_samples")})
    });
    json!({"id":id,"model":r.get::<String,_>("model"),
        "counts":counts.get(&id).cloned().unwrap_or_else(|| json!({})),"version":version,
        "compatible_revision":r.get::<Option<i32>,_>("compatible_revision"),
        "target_samples":r.get::<Option<i32>,_>("target_samples"),
        "version_id":r.get::<Option<Uuid>,_>("version_id"),"plan_id":r.get::<Option<Uuid>,_>("plan_id"),
        "status":r.get::<String,_>("status"),"manifest":r.get::<Value,_>("manifest"),
        "samples_sha256":r.get::<String,_>("samples_sha256"),"validation":r.get::<Value,_>("validation"),
        "drift":r.get::<Option<Value>,_>("drift"),"preview":r.get::<Option<Value>,_>("preview"),
        "sample_count":r.get::<i32,_>("sample_count"),"valid_count":r.get::<i32,_>("valid_count"),
        "created_at":r.get::<DateTime<Utc>,_>("created_at"),"decided_at":r.get::<Option<DateTime<Utc>>,_>("decided_at"),
        "excluded_at":r.get::<Option<DateTime<Utc>>,_>("excluded_at")})
}

pub async fn list_imports(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    auth::require_super_admin(&state, &headers).await?;
    let rows = sqlx::query(&format!(
        "{BATCH_SELECT} ORDER BY b.created_at DESC LIMIT 100"
    ))
    .fetch_all(&state.db)
    .await?;
    let ids: Vec<Uuid> = rows.iter().map(|r| r.get("id")).collect();
    let counts = sample_counts(&state.db, &ids).await?;
    Ok(Json(json!(
        rows.iter()
            .map(|r| batch_json(r, &counts))
            .collect::<Vec<_>>()
    )))
}

pub async fn get_import(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    auth::require_super_admin(&state, &headers).await?;
    Ok(Json(batch_view(&state.db, id).await?))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Merge,
    NewVersion,
    Reject,
}

/// Applies an admin decision to a pending batch. Returns the version it joined.
pub async fn decide(
    db: &PgPool,
    user_id: Uuid,
    id: Uuid,
    decision: Decision,
) -> Result<Option<Uuid>> {
    let mut tx = db.begin().await?;
    let batch = sqlx::query("SELECT * FROM model_baseline_batches WHERE id=$1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AppError::NotFound)?;
    if batch.get::<String, _>("status") != "pending" {
        return Err(AppError::Conflict("这个批次已经处理过".into()));
    }
    let manifest: bundle::Manifest = serde_json::from_value(batch.get("manifest"))
        .map_err(|_| AppError::Internal("stored manifest is invalid".into()))?;
    let version_id = match decision {
        Decision::Reject => {
            sqlx::query("UPDATE model_baseline_batches SET status='rejected',decided_by=$2,decided_at=now() WHERE id=$1")
                .bind(id)
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            return Ok(None);
        }
        Decision::Merge => {
            if batch
                .get::<Option<Value>, _>("drift")
                .is_some_and(|d| d["flagged"] == true)
            {
                return Err(AppError::Conflict(
                    "本批次的分布与现有版本明显不同，模型可能已更新，请作为新版本导入".into(),
                ));
            }
            // Re-check: the compatible version may have changed since the import.
            compatible_version(
                &mut tx,
                &manifest.model,
                &manifest.provider_only,
                manifest.pool_version,
                &manifest.reasoning,
            )
            .await?
            .filter(|current| {
                batch.get::<Value, _>("validation")["compatible_version_id"] == json!(current)
            })
            .ok_or_else(|| AppError::Conflict("没有可合并的兼容版本，请作为新版本导入".into()))?
        }
        Decision::NewVersion => {
            let family: String =
                sqlx::query_scalar("SELECT family FROM model_baseline_targets WHERE model=$1")
                    .bind(&manifest.model)
                    .fetch_optional(&mut *tx)
                    .await?
                    .ok_or_else(|| invalid("这个模型没有基准配置"))?;
            sqlx::query_scalar(
                "INSERT INTO model_baseline_versions(model,family,source,provider_only,pool_version,reasoning,
                 range_max,format_version,created_by) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) RETURNING id",
            )
            .bind(&manifest.model)
            .bind(family)
            .bind(&manifest.source)
            .bind(&manifest.provider_only)
            .bind(manifest.pool_version)
            .bind(&manifest.reasoning)
            .bind(manifest.range_max)
            .bind(manifest.format_version)
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?
        }
    };
    sqlx::query(
        "UPDATE model_baseline_batches SET status='merged',version_id=$2,decided_by=$3,decided_at=now() WHERE id=$1",
    )
    .bind(id)
    .bind(version_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    recompute(&mut tx, version_id).await?;
    tx.commit().await?;
    Ok(Some(version_id))
}

async fn decision_handler(
    state: AppState,
    headers: HeaderMap,
    id: Uuid,
    decision: Decision,
) -> Result<Json<Value>> {
    let admin = auth::require_super_admin(&state, &headers).await?;
    let version = decide(&state.db, admin.user_id, id, decision).await?;
    if let Some(version) = version {
        spawn_evaluation(&state.db, version);
    }
    let action = match decision {
        Decision::Merge => "platform.baseline_batch_merged",
        Decision::NewVersion => "platform.baseline_batch_new_version",
        Decision::Reject => "platform.baseline_batch_rejected",
    };
    audit(
        &state.db,
        admin.account_id,
        admin.user_id,
        action,
        "model_baseline_batch",
        id,
        json!({"version_id":version}),
    )
    .await;
    Ok(Json(json!({"id":id,"version_id":version})))
}

pub async fn merge(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    decision_handler(state, headers, id, Decision::Merge).await
}

pub async fn new_version(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    decision_handler(state, headers, id, Decision::NewVersion).await
}

pub async fn reject(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    decision_handler(state, headers, id, Decision::Reject).await
}

pub async fn exclude_batch(db: &PgPool, id: Uuid) -> Result<Uuid> {
    let mut tx = db.begin().await?;
    let version_id: Uuid = sqlx::query_scalar(
        "UPDATE model_baseline_batches SET excluded_at=now() WHERE id=$1 AND status='merged' AND excluded_at IS NULL
         RETURNING version_id",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::Conflict("只能排除已合并且未排除的批次".into()))?;
    recompute(&mut tx, version_id).await?;
    tx.commit().await?;
    Ok(version_id)
}

pub async fn exclude(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let admin = auth::require_super_admin(&state, &headers).await?;
    let version = exclude_batch(&state.db, id).await?;
    spawn_evaluation(&state.db, version);
    audit(
        &state.db,
        admin.account_id,
        admin.user_id,
        "platform.baseline_batch_excluded",
        "model_baseline_batch",
        id,
        json!({"version_id":version}),
    )
    .await;
    Ok(Json(json!({"id":id,"version_id":version})))
}

#[cfg(test)]
mod lookup_tests {
    use super::*;

    #[test]
    fn matches_price_table_names_and_lists_provider_slugs() {
        let models = json!({"data":[
            {"id":"anthropic/claude-sonnet-5.5","name":"Anthropic: Claude Sonnet 5.5","supported_parameters":["reasoning","max_tokens"]},
            {"id":"anthropic/claude-sonnet-5.5:batch","name":"batch"},
            {"id":"openai/gpt-6.1-sol","name":"OpenAI: GPT-6.1 Sol","supported_parameters":["max_tokens"]}
        ]});
        let found = matching_ids(&models, "claude-sonnet-5-5");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0]["id"], "anthropic/claude-sonnet-5.5");
        assert_eq!(found[0]["supports_reasoning"], true);
        assert_eq!(
            matching_ids(&models, "gpt-6.1-sol")[0]["supports_reasoning"],
            false
        );
        assert!(matching_ids(&models, "claude-sonnet-5.5").is_empty());
        let endpoints = json!({"data":{"endpoints":[
            {"provider_name":"Anthropic","tag":"anthropic","quantization":"unknown"},
            {"provider_name":"Anthropic","tag":"anthropic/fast"},
            {"provider_name":"Amazon Bedrock","tag":"amazon-bedrock/us-east-1"}
        ]}});
        let slugs: Vec<_> = providers(&endpoints)
            .iter()
            .map(|p| p["slug"].clone())
            .collect();
        assert_eq!(slugs, vec![json!("anthropic"), json!("amazon-bedrock")]);
    }
}
