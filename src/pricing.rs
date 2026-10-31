use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    error::{AppError, Result},
    state::AppState,
};

#[cfg(test)]
#[path = "pricing_db_tests.rs"]
mod db_tests;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ModelPrice {
    /// Exact upstream model ID. The legacy JSON field name is retained for API compatibility.
    pub pattern: String,
    pub label: String,
    /// USD per 1 million input tokens (non-cached portion).
    pub input_per_1m: f64,
    /// USD per 1 million output tokens.
    pub output_per_1m: f64,
    /// USD per 1 million cached-read tokens.
    pub cached_read_per_1m: f64,
    /// "openrouter" when auto-synced, null when manually entered.
    pub synced_from: Option<String>,
}

impl ModelPrice {
    pub fn estimate(&self, input: i64, output: i64, cached: i64) -> f64 {
        let cached = cached.clamp(0, input.max(0));
        (input.saturating_sub(cached).max(0) as f64 * self.input_per_1m
            + output.max(0) as f64 * self.output_per_1m
            + cached as f64 * self.cached_read_per_1m)
            / 1_000_000.0
    }
}

pub async fn load(db: &PgPool) -> Result<Vec<ModelPrice>> {
    Ok(sqlx::query_as(
        "SELECT pattern,label,input_per_1m,output_per_1m,cached_read_per_1m,price_source AS synced_from
         FROM model_catalog WHERE input_per_1m IS NOT NULL AND strpos(pattern,'*')=0 ORDER BY pattern"
    ).fetch_all(db).await?)
}

/// How often the gateway rereads prices, for edits it was not told about.
const REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// Replaces the gateway's cached prices.
pub fn set(state: &AppState, prices: Vec<ModelPrice>) {
    *state
        .prices
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(std::sync::Arc::new(prices));
}

/// Rereads the prices into the cache after an edit. A failed read keeps the
/// current prices; the next scheduled refresh tries again.
pub async fn refresh(state: &AppState) {
    match load(&state.db).await {
        Ok(prices) => set(state, prices),
        Err(error) => eprintln!("model price refresh failed: {error}"),
    }
}

/// Keeps the cached prices current.
pub fn start_refresh(state: AppState) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(REFRESH_INTERVAL).await;
            refresh(&state).await;
        }
    });
}

pub fn validate(price: &ModelPrice) -> Result<()> {
    if !crate::model_rules::concrete(&price.pattern) || price.label.trim().is_empty() {
        return Err(AppError::BadRequest(
            "价格必须绑定具体模型 ID，不支持前缀规则".into(),
        ));
    }
    if [
        price.input_per_1m,
        price.output_per_1m,
        price.cached_read_per_1m,
    ]
    .iter()
    .any(|p| !p.is_finite() || *p < 0.0)
    {
        return Err(AppError::BadRequest("价格必须是非负的有效数字".into()));
    }
    Ok(())
}

pub async fn save(db: &PgPool, prices: &[ModelPrice]) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    for price in prices {
        validate(price)?;
        if !seen.insert(&price.pattern) {
            return Err(AppError::Conflict("具体模型价格重复".into()));
        }
    }
    let mut tx = db.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(324021)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE model_catalog SET input_per_1m=NULL,output_per_1m=NULL,cached_read_per_1m=NULL,price_source=NULL WHERE input_per_1m IS NOT NULL")
        .execute(&mut *tx).await?;
    for price in prices {
        write_price(&mut tx, price, false).await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn write_price(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    price: &ModelPrice,
    only_unpriced: bool,
) -> Result<()> {
    let result = sqlx::query(
        "INSERT INTO model_catalog(pattern,label,input_per_1m,output_per_1m,cached_read_per_1m,price_source)
         VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT(pattern) DO UPDATE
         SET input_per_1m=$3,output_per_1m=$4,cached_read_per_1m=$5,price_source=$6,updated_at=now()
         WHERE NOT $7 OR model_catalog.input_per_1m IS NULL"
    ).bind(&price.pattern).bind(&price.label).bind(price.input_per_1m).bind(price.output_per_1m)
        .bind(price.cached_read_per_1m).bind(&price.synced_from).bind(only_unpriced).execute(&mut **tx).await?;
    if result.rows_affected() == 0 {
        return Err(AppError::Conflict("该模型已定价，请刷新后编辑".into()));
    }
    Ok(())
}

pub async fn sync_one(db: &PgPool, fetched: &[ModelPrice], target: &str) -> Result<usize> {
    let Some(price) = lookup(fetched, target).filter(|price| validate(price).is_ok()) else {
        return Ok(0);
    };
    Ok(sqlx::query("UPDATE model_catalog SET input_per_1m=$2,output_per_1m=$3,cached_read_per_1m=$4,price_source=$5,updated_at=now() WHERE pattern=$1")
        .bind(&price.pattern).bind(price.input_per_1m).bind(price.output_per_1m).bind(price.cached_read_per_1m).bind(&price.synced_from)
        .execute(db).await?.rows_affected() as usize)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PriceRates {
    pub input_per_1m: f64,
    pub output_per_1m: f64,
    pub cached_read_per_1m: f64,
}

impl PriceRates {
    fn from_price(price: &ModelPrice) -> Self {
        Self {
            input_per_1m: price.input_per_1m,
            output_per_1m: price.output_per_1m,
            cached_read_per_1m: price.cached_read_per_1m,
        }
    }

    fn values(&self) -> [f64; 3] {
        [
            self.input_per_1m,
            self.output_per_1m,
            self.cached_read_per_1m,
        ]
    }

    fn equivalent(&self, other: &Self) -> bool {
        // Ignore JSON/float round-trip noise, but never equate a nonzero rate with free.
        self.values()
            .into_iter()
            .zip(other.values())
            .all(|(a, b)| a == b || (a - b).abs() <= 1e-12 * a.abs().max(b.abs()))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PriceSyncChange {
    pub id: Uuid,
    pub pattern: String,
    pub label: String,
    pub updated_at: DateTime<Utc>,
    pub current: Option<PriceRates>,
    pub reference: PriceRates,
}

#[derive(sqlx::FromRow)]
struct CatalogPrice {
    id: Uuid,
    pattern: String,
    label: String,
    updated_at: DateTime<Utc>,
    input_per_1m: Option<f64>,
    output_per_1m: Option<f64>,
    cached_read_per_1m: Option<f64>,
}

impl CatalogPrice {
    fn rates(&self) -> Option<PriceRates> {
        Some(PriceRates {
            input_per_1m: self.input_per_1m?,
            output_per_1m: self.output_per_1m?,
            cached_read_per_1m: self.cached_read_per_1m?,
        })
    }
}

pub async fn preview_sync(db: &PgPool, fetched: &[ModelPrice]) -> Result<Vec<PriceSyncChange>> {
    let rows = sqlx::query_as::<_, CatalogPrice>(
        "SELECT id,pattern,label,updated_at,input_per_1m,output_per_1m,cached_read_per_1m
         FROM model_catalog WHERE strpos(pattern,'*')=0 ORDER BY label,pattern",
    )
    .fetch_all(db)
    .await?;
    let mut changes = Vec::new();
    for row in rows {
        let Some(price) = lookup(fetched, &row.pattern).filter(|price| validate(price).is_ok())
        else {
            continue;
        };
        let reference = PriceRates::from_price(price);
        let current = row.rates();
        if current
            .as_ref()
            .is_some_and(|rates| rates.equivalent(&reference))
        {
            continue;
        }
        changes.push(PriceSyncChange {
            id: row.id,
            pattern: row.pattern,
            label: row.label,
            updated_at: row.updated_at,
            current,
            reference,
        });
    }
    Ok(changes)
}

pub fn validate_sync_changes(changes: &[PriceSyncChange]) -> Result<()> {
    if changes.is_empty() {
        return Err(AppError::BadRequest(
            "请至少选择一个有价格差异的模型".into(),
        ));
    }
    let mut ids = std::collections::HashSet::new();
    let mut names = std::collections::HashSet::new();
    for change in changes {
        if !crate::model_rules::concrete(&change.pattern)
            || !ids.insert(change.id)
            || !names.insert(&change.pattern)
            || std::iter::once(&change.reference)
                .chain(change.current.iter())
                .flat_map(PriceRates::values)
                .any(|rate| !rate.is_finite() || rate < 0.0)
        {
            return Err(AppError::BadRequest(
                "同步列表包含重复模型或无效价格".into(),
            ));
        }
    }
    Ok(())
}

pub async fn apply_sync(
    db: &PgPool,
    changes: &[PriceSyncChange],
    fetched: &[ModelPrice],
) -> Result<usize> {
    validate_sync_changes(changes)?;
    let reference_conflict =
        || AppError::Conflict("参考价已变化或不可用，请重新获取差异后同步".into());
    for change in changes {
        let price = lookup(fetched, &change.pattern)
            .filter(|price| validate(price).is_ok())
            .ok_or_else(reference_conflict)?;
        if !change.reference.equivalent(&PriceRates::from_price(price)) {
            return Err(reference_conflict());
        }
    }
    let ids: Vec<Uuid> = changes.iter().map(|change| change.id).collect();
    let mut tx = db.begin().await?;
    // Lock the full selection before checking snapshots or writing any price.
    let rows = sqlx::query_as::<_, CatalogPrice>(
        "SELECT id,pattern,label,updated_at,input_per_1m,output_per_1m,cached_read_per_1m
         FROM model_catalog WHERE id=ANY($1) ORDER BY id FOR UPDATE",
    )
    .bind(&ids)
    .fetch_all(&mut *tx)
    .await?;
    let catalog_conflict =
        || AppError::Conflict("模型价格或目录已被修改，请重新获取差异后同步".into());
    if rows.len() != changes.len() {
        return Err(catalog_conflict());
    }
    for change in changes {
        let row = rows
            .iter()
            .find(|row| row.id == change.id)
            .ok_or_else(catalog_conflict)?;
        let same_current = match (row.rates(), &change.current) {
            (None, None) => true,
            (Some(actual), Some(expected)) => actual.equivalent(expected),
            _ => false,
        };
        if row.pattern != change.pattern || row.updated_at != change.updated_at || !same_current {
            return Err(catalog_conflict());
        }
    }
    for change in changes {
        let price = lookup(fetched, &change.pattern).ok_or_else(reference_conflict)?;
        sqlx::query("UPDATE model_catalog SET input_per_1m=$2,output_per_1m=$3,cached_read_per_1m=$4,price_source='openrouter',updated_at=now() WHERE id=$1")
            .bind(change.id).bind(price.input_per_1m).bind(price.output_per_1m).bind(price.cached_read_per_1m)
            .execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(changes.len())
}

pub async fn add(db: &PgPool, mut price: ModelPrice) -> Result<Vec<ModelPrice>> {
    price.pattern = price.pattern.trim().to_owned();
    price.label = price.label.trim().to_owned();
    validate(&price)?;
    let mut tx = db.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(324021)")
        .execute(&mut *tx)
        .await?;
    write_price(&mut tx, &price, true).await?;
    tx.commit().await?;
    load(db).await
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct UnpricedModel {
    pub model: String,
    pub requests: i64,
    pub last_seen_at: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct UnpricedModels {
    pub since: DateTime<Utc>,
    pub models: Vec<UnpricedModel>,
}

pub async fn recent_unpriced(db: &PgPool) -> Result<UnpricedModels> {
    let until = Utc::now();
    let since = until - Duration::days(30);
    let prices = load(db).await?;
    let mut models = sqlx::query_as::<_, UnpricedModel>(
        "SELECT upstream_model AS model,count(*) AS requests,max(created_at) AS last_seen_at
         FROM usage_events WHERE created_at >= $1 AND created_at <= $2
         AND upstream_model IS NOT NULL AND btrim(upstream_model) <> ''
         -- A relayed request is counted once, on the source key's own row.
         AND relayed_event_id IS NULL
         GROUP BY upstream_model ORDER BY requests DESC,last_seen_at DESC,model ASC",
    )
    .bind(since)
    .bind(until)
    .fetch_all(db)
    .await?;
    models.retain(|model| lookup(&prices, &model.model).is_none());
    Ok(UnpricedModels { since, models })
}

pub fn lookup<'a>(prices: &'a [ModelPrice], model: &str) -> Option<&'a ModelPrice> {
    prices.iter().find(|price| price.pattern == model)
}

#[derive(Deserialize)]
struct OrModel {
    id: String,
    name: Option<String>,
    pricing: Option<OrPricing>,
}

#[derive(Deserialize, Default)]
struct OrPricing {
    prompt: Option<String>,
    completion: Option<String>,
    #[serde(rename = "input_cache_read")]
    input_cache_read: Option<String>,
    /// Time-of-day prices, such as DeepSeek's peak hours. Fields an override
    /// leaves out keep the base price.
    #[serde(default)]
    overrides: Vec<OrPricing>,
}

#[derive(Deserialize)]
struct OrEndpoint {
    provider_name: Option<String>,
    pricing: Option<OrPricing>,
}

/// OpenRouter providers that are the vendor's own API, keyed by model id
/// prefix. Third-party hosts often list these models far below the official
/// price, so only the first-party endpoint is used as the reference.
const FIRST_PARTY: &[(&str, &str)] = &[
    ("deepseek", "DeepSeek"),
    ("moonshotai", "Moonshot AI"),
    ("z-ai", "Z.AI"),
    ("qwen", "Alibaba"),
    ("minimax", "Minimax"),
    ("bytedance-seed", "Seed"),
];

fn first_party(id: &str) -> Option<&'static str> {
    // "~vendor/name-latest" aliases follow the vendor's newest model.
    let (vendor, _) = id.trim_start_matches('~').split_once('/')?;
    FIRST_PARTY
        .iter()
        .find(|(prefix, _)| *prefix == vendor)
        .map(|(_, provider)| *provider)
}

/// The cheapest first-party endpoint, or none when the vendor does not serve
/// the model itself. Regional endpoints such as Alibaba's us-east-1 cost more.
fn first_party_pricing(provider: &str, endpoints: Vec<OrEndpoint>) -> Option<OrPricing> {
    endpoints
        .into_iter()
        .filter(|endpoint| endpoint.provider_name.as_deref() == Some(provider))
        .filter_map(|endpoint| endpoint.pricing)
        .filter_map(|pricing| reference_rates(&pricing).map(|rates| (rates, pricing)))
        .min_by(|(a, _), (b, _)| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])))
        .map(|(_, pricing)| pricing)
}

/// A failed lookup leaves the model unpriced rather than falling back to a
/// third-party list price.
async fn official_pricing(
    client: reqwest::Client,
    index: usize,
    id: String,
    provider: String,
) -> (usize, Option<OrPricing>) {
    let endpoints = fetch_endpoints(&client, &id).await.unwrap_or_default();
    (index, first_party_pricing(&provider, endpoints))
}

async fn fetch_endpoints(
    client: &reqwest::Client,
    id: &str,
) -> std::result::Result<Vec<OrEndpoint>, String> {
    let resp: serde_json::Value = client
        .get(format!(
            "https://openrouter.ai/api/v1/models/{id}/endpoints"
        ))
        .header("User-Agent", "meta-router/1.0")
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())
        .map(|response| {
            crate::gateway::read_json_limited(response, crate::gateway::MAX_METADATA_BODY)
        })?
        .await?;
    serde_json::from_value(
        resp.pointer("/data/endpoints")
            .cloned()
            .ok_or_else(|| "missing endpoint data".to_string())?,
    )
    .map_err(|e| e.to_string())
}

/// Convert an OpenRouter id ("anthropic/claude-sonnet-5.5") to the model name
/// clients send upstream ("claude-sonnet-5-5"), which is what usage rows record.
pub(crate) fn native_model_name(id: &str) -> Option<String> {
    let (vendor, name) = id.split_once('/').unwrap_or(("", id));
    let name = name.trim().to_owned();
    if name.is_empty() {
        return None;
    }
    // Anthropic's native API ids use dashes in version numbers.
    Some(if vendor == "anthropic" {
        name.replace('.', "-")
    } else {
        name
    })
}

#[cfg(test)]
fn best_match<'a>(fetched: &'a [ModelPrice], model: &str) -> Option<&'a ModelPrice> {
    lookup(fetched, model)
}

fn parse_price(s: &str) -> Option<f64> {
    s.trim()
        .parse::<f64>()
        .ok()
        .filter(|&v| v.is_finite() && v >= 0.0)
}

/// Per-1M rates at the most expensive time of day. A single stored price
/// cannot follow peak hours, so the peak rate keeps cost limits on the safe side.
fn reference_rates(pricing: &OrPricing) -> Option<[f64; 3]> {
    let rates = |o: &OrPricing| -> Option<[f64; 3]> {
        let field = |own: &Option<String>, base: &Option<String>| {
            parse_price(own.as_deref().or(base.as_deref())?).map(|price| price * 1_000_000.0)
        };
        let rates = [
            field(&o.prompt, &pricing.prompt)?,
            field(&o.completion, &pricing.completion)?,
            field(&o.input_cache_read, &pricing.input_cache_read)?,
        ];
        rates.iter().all(|rate| rate.is_finite()).then_some(rates)
    };
    let mut peak = rates(pricing)?;
    for o in &pricing.overrides {
        let window = rates(o)?;
        if window[0]
            .total_cmp(&peak[0])
            .then(window[1].total_cmp(&peak[1]))
            .is_gt()
        {
            peak = window;
        }
    }
    Some(peak)
}

/// Fetch the OpenRouter model list and convert to ModelPrice entries.
/// Returns only models that have valid pricing data.
pub async fn fetch_openrouter() -> std::result::Result<Vec<ModelPrice>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;

    let resp: serde_json::Value = client
        .get("https://openrouter.ai/api/v1/models")
        .header("User-Agent", "meta-router/1.0")
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())
        .map(|response| {
            crate::gateway::read_json_limited(response, crate::gateway::MAX_METADATA_BODY)
        })?
        .await?;

    let mut models: Vec<OrModel> = serde_json::from_value(
        resp.get("data")
            .cloned()
            .ok_or_else(|| "missing model data".to_string())?,
    )
    .map_err(|e| e.to_string())?;

    use futures_util::StreamExt;
    let targets: Vec<_> = models
        .iter()
        .enumerate()
        .filter(|(_, m)| !m.id.contains(':'))
        .filter_map(|(i, m)| {
            first_party(&m.id).map(|provider| (i, m.id.clone(), provider.to_owned()))
        })
        .collect();
    let official: Vec<_> = futures_util::stream::iter(targets)
        .map(|(i, id, provider)| official_pricing(client.clone(), i, id, provider))
        .buffer_unordered(8)
        .collect()
        .await;
    for (i, pricing) in official {
        models[i].pricing = pricing;
    }

    Ok(reference_prices(models))
}

/// Keeps one exact reference price per native model name. Names that several
/// OpenRouter IDs normalize to with different rates are ambiguous and skipped,
/// so the result does not depend on response order.
fn reference_prices(models: Vec<OrModel>) -> Vec<ModelPrice> {
    let mut out: Vec<ModelPrice> = Vec::new();
    let mut index = std::collections::HashMap::new();
    let mut ambiguous = std::collections::HashSet::new();
    for m in models {
        let pricing = match &m.pricing {
            Some(p) => p,
            None => continue,
        };
        // Missing reference fields are unknown, never implicitly free.
        let Some([input_per_1m, output_per_1m, cached_per_1m]) = reference_rates(pricing) else {
            continue;
        };

        // Skip variants such as ":free" / ":batch" / ":extended".
        if m.id.contains(':') {
            continue;
        }
        let Some(pattern) = native_model_name(&m.id) else {
            continue;
        };
        if !crate::model_rules::concrete(&pattern) {
            continue;
        }
        // Drop the "Vendor: " prefix OpenRouter adds to display names.
        let label = m
            .name
            .as_deref()
            .map(|n| n.split_once(": ").map_or(n, |(_, rest)| rest).trim())
            .filter(|n| !n.is_empty())
            .unwrap_or(&pattern)
            .to_string();
        let price = ModelPrice {
            pattern,
            label,
            input_per_1m,
            output_per_1m,
            cached_read_per_1m: cached_per_1m,
            synced_from: Some("openrouter".into()),
        };
        if let Some(&existing) = index.get(&price.pattern) {
            let existing: &ModelPrice = &out[existing];
            if !PriceRates::from_price(existing).equivalent(&PriceRates::from_price(&price)) {
                ambiguous.insert(price.pattern);
            }
            continue;
        }
        index.insert(price.pattern.clone(), out.len());
        out.push(price);
    }
    out.retain(|price| !ambiguous.contains(&price.pattern));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn price_comparison_ignores_float_noise_but_preserves_nonzero_rates() {
        let rates = |input| PriceRates {
            input_per_1m: input,
            output_per_1m: 0.0,
            cached_read_per_1m: 0.0,
        };
        assert!(rates(0.1 + 0.2).equivalent(&rates(0.3)));
        assert!(!rates(0.0).equivalent(&rates(1e-15)));
        assert!(!rates(0.3).equivalent(&rates(0.31)));
    }

    fn p(pattern: &str) -> ModelPrice {
        ModelPrice {
            pattern: pattern.into(),
            label: pattern.into(),
            input_per_1m: 0.0,
            output_per_1m: 0.0,
            cached_read_per_1m: 0.0,
            synced_from: None,
        }
    }

    #[test]
    fn strips_vendor_prefix_and_normalizes_anthropic_versions() {
        assert_eq!(
            native_model_name("openai/gpt-4.1").as_deref(),
            Some("gpt-4.1")
        );
        assert_eq!(
            native_model_name("anthropic/claude-sonnet-5.5").as_deref(),
            Some("claude-sonnet-5-5")
        );
        assert_eq!(native_model_name("gpt-4o").as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn exact_prices_never_inherit_prefix_or_case_variants() {
        let fetched = vec![p("gpt-4.1"), p("gpt-4.1-mini")];
        assert_eq!(best_match(&fetched, "gpt-4.1").unwrap().pattern, "gpt-4.1");
        assert!(best_match(&fetched, "gpt-4.1-2026").is_none());
        assert!(best_match(&fetched, "GPT-4.1").is_none());
        assert!(validate(&p("gpt-*")).is_err());
    }

    #[test]
    fn ambiguous_or_unnamed_references_never_depend_on_order() {
        let model = |id: &str, name: &str, prompt: &str| OrModel {
            id: id.into(),
            name: Some(name.into()),
            pricing: Some(OrPricing {
                prompt: Some(prompt.into()),
                completion: Some("0".into()),
                input_cache_read: Some("0".into()),
                ..Default::default()
            }),
        };
        let prices = reference_prices(vec![
            model("anthropic/x-3.5", "Anthropic: X", "0.000001"),
            model("anthropic/x-3-5", "Anthropic: X", "0.000002"),
            model("anthropic/y-1.5", "Anthropic: Y", "0.000001"),
            model("anthropic/y-1-5", "Anthropic: Y", "0.000001"),
            model("vendor/unnamed", "Vendor: ", "0.000001"),
        ]);
        let names: Vec<_> = prices.iter().map(|price| price.pattern.as_str()).collect();
        assert_eq!(names, ["y-1-5", "unnamed"]);
        assert_eq!(prices[1].label, "unnamed");
    }

    #[test]
    fn incomplete_reference_prices_are_not_free() {
        let mut reference = OrPricing {
            prompt: Some("0.000002".into()),
            completion: Some("0".into()),
            input_cache_read: None,
            ..Default::default()
        };
        assert!(reference_rates(&reference).is_none());
        reference.input_cache_read = Some("0".into());
        assert_eq!(reference_rates(&reference), Some([2.0, 0.0, 0.0]));
        reference.completion = Some("NaN".into());
        assert!(reference_rates(&reference).is_none());
    }

    #[test]
    fn peak_hours_set_the_reference_price() {
        let pricing: OrPricing = serde_json::from_value(serde_json::json!({
            "prompt": "0.00000066", "completion": "0.00000198", "input_cache_read": "0.000000022",
            "overrides": [
                {"utc_days": ["saturday"], "prompt": "0.00000066"},
                {"utc_start": 100, "utc_end": 400, "prompt": "0.00000132",
                 "completion": "0.00000396", "input_cache_read": "0.000000044"},
            ],
        }))
        .unwrap();
        let rates = reference_rates(&pricing).unwrap();
        assert!((rates[0] - 1.32).abs() < 1e-9);
        assert!((rates[1] - 3.96).abs() < 1e-9);
        assert!((rates[2] - 0.044).abs() < 1e-9);

        let broken: OrPricing = serde_json::from_value(serde_json::json!({
            "prompt": "0.000001", "completion": "0", "input_cache_read": "0",
            "overrides": [{"prompt": "NaN"}],
        }))
        .unwrap();
        assert!(reference_rates(&broken).is_none());
    }

    #[test]
    fn domestic_vendors_use_only_their_own_endpoint() {
        assert_eq!(first_party("moonshotai/kimi-k3"), Some("Moonshot AI"));
        assert_eq!(first_party("qwen/qwen3.7-max"), Some("Alibaba"));
        assert_eq!(first_party("~z-ai/glm-latest"), Some("Z.AI"));
        assert_eq!(first_party("openai/gpt-5"), None);
        assert_eq!(first_party("deepseek"), None);

        let endpoint = |provider: &str, prompt: &str| OrEndpoint {
            provider_name: Some(provider.into()),
            pricing: Some(OrPricing {
                prompt: Some(prompt.into()),
                completion: Some("0".into()),
                input_cache_read: Some("0".into()),
                ..Default::default()
            }),
        };
        let pricing = first_party_pricing(
            "Alibaba",
            vec![
                endpoint("Wafer", "0.0000001"),
                endpoint("Alibaba", "0.000002"),
                endpoint("Alibaba", "0.000001475"),
            ],
        )
        .unwrap();
        assert_eq!(pricing.prompt.as_deref(), Some("0.000001475"));
        assert!(
            first_party_pricing("DeepSeek", vec![endpoint("StreamLake", "0.0000002")]).is_none()
        );
    }
}
