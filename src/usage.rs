//! Usage analysis: aggregates real gateway traffic recorded in `usage_events`.
//! Synthetic monitor probes live in `channel_monitor_runs` and never appear here.

use crate::{
    auth,
    error::{AppError, Result},
    request_diagnostics::Diagnostics,
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use chrono::{DateTime, Duration, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::FromRow;
use uuid::Uuid;

// Shared WHERE clause; binds $1..$8 in the order used by `bind_filters!`.
const FILTER: &str = "e.account_id=$1 AND e.created_at >= $2
  AND ($3::text IS NULL OR e.model=$3)
  AND ($4::text IS NULL OR e.protocol=$4)
  AND ($5::text IS NULL OR ($5='success' AND e.status BETWEEN 200 AND 299) OR ($5='error' AND e.status >= 400))
  AND ($6::uuid IS NULL OR e.channel_id=$6)
  AND ($7::text IS NULL OR strpos(lower(e.model||' '||e.protocol||' '||e.status::text), lower($7)) > 0)
  AND ($8::uuid IS NULL OR e.virtual_key_id=$8)";

macro_rules! bind_filters {
    ($query:expr, $filters:expr) => {
        $query
            .bind($filters.account_id)
            .bind($filters.since)
            .bind($filters.model.as_deref())
            .bind($filters.protocol.as_deref())
            .bind($filters.status)
            .bind($filters.channel_id)
            .bind($filters.search.as_deref())
            .bind($filters.key_id)
    };
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
pub enum UsageRange {
    #[default]
    #[serde(rename = "24h")]
    Day,
    #[serde(rename = "7d")]
    Week,
    #[serde(rename = "30d")]
    Month,
    /// From the first matching request; see `all_time_plan`.
    #[serde(rename = "all")]
    All,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsageStatus {
    Success,
    Error,
}

#[derive(Debug, Deserialize)]
pub struct UsageQuery {
    range: Option<UsageRange>,
    model: Option<String>,
    protocol: Option<String>,
    status: Option<UsageStatus>,
    channel_id: Option<Uuid>,
    key_id: Option<Uuid>,
    search: Option<String>,
    /// Browser offset from UTC (local minus UTC), used to align day buckets.
    utc_offset_minutes: Option<i32>,
    page: Option<i64>,
    limit: Option<i64>,
}

struct Filters {
    account_id: Uuid,
    since: DateTime<Utc>,
    model: Option<String>,
    protocol: Option<String>,
    status: Option<&'static str>,
    channel_id: Option<Uuid>,
    search: Option<String>,
    key_id: Option<Uuid>,
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

pub(crate) struct BucketPlan {
    pub(crate) start: DateTime<Utc>,
    pub(crate) step_seconds: i64,
    count: i64,
}

/// Buckets end at the current local hour (24h) or day (7d/30d) so the first
/// bucket is never partial and every bucket lines up with the user's clock.
pub(crate) fn bucket_plan(
    range: UsageRange,
    now: DateTime<Utc>,
    utc_offset_minutes: i32,
) -> BucketPlan {
    let offset = Duration::minutes(utc_offset_minutes.clamp(-840, 840).into());
    let local = (now + offset).naive_utc();
    let (step_seconds, count, truncated) = match range {
        UsageRange::Day => (
            3600,
            24,
            local
                .date()
                .and_hms_opt(local.hour(), 0, 0)
                .unwrap_or(local),
        ),
        UsageRange::Week => (
            86_400,
            7,
            local.date().and_hms_opt(0, 0, 0).unwrap_or(local),
        ),
        UsageRange::Month => (
            86_400,
            30,
            local.date().and_hms_opt(0, 0, 0).unwrap_or(local),
        ),
        UsageRange::All => return all_time_plan(now, now, utc_offset_minutes),
    };
    let start_local = truncated - Duration::seconds(step_seconds * (count - 1));
    BucketPlan {
        start: start_local.and_utc() - offset,
        step_seconds,
        count,
    }
}
/// All-time buckets keep the chart near 60 bars: daily, then weekly, then
/// 30-day steps. The last bucket ends at the end of the user's current day.
pub(crate) fn all_time_plan(
    first: DateTime<Utc>,
    now: DateTime<Utc>,
    utc_offset_minutes: i32,
) -> BucketPlan {
    let offset = Duration::minutes(utc_offset_minutes.clamp(-840, 840).into());
    let first_day = (first.min(now) + offset).date_naive();
    let today = (now + offset).date_naive();
    let days = (today - first_day).num_days() + 1;
    let step_days = if days <= 60 {
        1
    } else if days <= 420 {
        7
    } else {
        30
    };
    let count = (days + step_days - 1) / step_days;
    let end = (today + Duration::days(1))
        .and_hms_opt(0, 0, 0)
        .unwrap_or_default();
    BucketPlan {
        start: (end - Duration::days(step_days * count)).and_utc() - offset,
        step_seconds: step_days * 86_400,
        count,
    }
}

impl UsageQuery {
    fn filters(&self, account_id: Uuid, since: DateTime<Utc>) -> Filters {
        Filters {
            account_id,
            since,
            model: non_empty(self.model.clone()),
            protocol: non_empty(self.protocol.clone()),
            status: self.status.map(|status| match status {
                UsageStatus::Success => "success",
                UsageStatus::Error => "error",
            }),
            channel_id: self.channel_id,
            search: non_empty(self.search.clone()),
            key_id: self.key_id,
        }
    }

    async fn plan(&self, state: &AppState, account_id: Uuid) -> Result<BucketPlan> {
        let range = self.range.unwrap_or_default();
        let offset = self.utc_offset_minutes.unwrap_or(0);
        let now = Utc::now();
        if range != UsageRange::All {
            return Ok(bucket_plan(range, now, offset));
        }
        // Start at the first request under the same filters, so a new key or
        // channel does not show a long run of empty buckets.
        let filters = self.filters(account_id, DateTime::<Utc>::UNIX_EPOCH);
        let sql = format!("SELECT MIN(e.created_at) FROM usage_events e WHERE {FILTER}");
        let first: Option<DateTime<Utc>> = bind_filters!(sqlx::query_scalar(&sql), filters)
            .fetch_one(&state.db)
            .await?;
        Ok(all_time_plan(first.unwrap_or(now), now, offset))
    }
}

#[derive(Serialize, FromRow)]
pub struct UsageView {
    id: Uuid,
    channel_id: Option<Uuid>,
    channel_name: Option<String>,
    virtual_key_id: Option<Uuid>,
    key_name: Option<String>,
    key_revoked: Option<bool>,
    model: String,
    upstream_model: Option<String>,
    price_snapshot: Option<Value>,
    cost_usd: Option<f64>,
    pricing_status: String,
    protocol: String,
    status: i32,
    ttfb_ms: Option<i32>,
    first_token_ms: Option<i32>,
    stream: Option<bool>,
    duration_ms: i32,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cached_tokens: Option<i64>,
    created_at: DateTime<Utc>,
    // 'monitor' marks checks another account ran through a VAIR key channel on this account's key.
    source: String,
}

const EVENT_COLUMNS: &str = "e.id,e.channel_id,c.name AS channel_name,e.model,e.protocol,e.status,
  e.ttfb_ms,e.first_token_ms,e.stream,e.duration_ms,e.input_tokens,e.output_tokens,e.cached_tokens,e.created_at,e.upstream_model,e.price_snapshot,e.cost_usd,e.pricing_status,
  e.virtual_key_id,k.name AS key_name,(k.revoked_at IS NOT NULL) AS key_revoked,e.source
  FROM usage_events e
  LEFT JOIN channels c ON c.id=e.channel_id AND c.account_id=e.account_id
  LEFT JOIN virtual_api_keys k ON k.id=e.virtual_key_id AND k.account_id=e.account_id";

/// Output performance: medians of time to first output and output speed over
/// successful streamed requests (only streams time the first token), and of
/// total duration over successful requests. Short answers say little about speed.
pub(crate) const PERFORMANCE_COLUMNS: &str = "percentile_cont(0.5) WITHIN GROUP (ORDER BY e.first_token_ms)
    FILTER (WHERE e.stream AND e.status BETWEEN 200 AND 299 AND e.first_token_ms IS NOT NULL)::double precision AS p50_first_token_ms,
  percentile_cont(0.5) WITHIN GROUP (ORDER BY e.duration_ms)
    FILTER (WHERE e.status BETWEEN 200 AND 299)::double precision AS p50_duration_ms,
  percentile_cont(0.5) WITHIN GROUP (ORDER BY e.output_tokens*1000.0/(e.duration_ms-e.first_token_ms))
    FILTER (WHERE e.stream AND e.status BETWEEN 200 AND 299 AND e.output_tokens>=32 AND e.first_token_ms IS NOT NULL AND e.duration_ms>e.first_token_ms)::double precision AS p50_output_tps,
  COUNT(*) FILTER (WHERE e.stream AND e.status BETWEEN 200 AND 299 AND e.output_tokens>=32 AND e.first_token_ms IS NOT NULL AND e.duration_ms>e.first_token_ms)::bigint AS speed_samples";

#[derive(Serialize, FromRow, Default)]
struct Performance {
    p50_first_token_ms: Option<f64>,
    p50_duration_ms: Option<f64>,
    p50_output_tps: Option<f64>,
    speed_samples: i64,
}

// Restrict coverage to requests with input so failed requests without usage do not
// dilute cache reporting. Legacy zero values remain unknown, not reported misses.
const CACHE_COLUMNS: &str = "COUNT(*) FILTER (WHERE e.input_tokens > 0 AND e.cache_usage_reported IS TRUE)::bigint AS cache_reported_requests,
  COUNT(*) FILTER (WHERE e.input_tokens > 0 AND e.cache_usage_reported IS FALSE)::bigint AS cache_unreported_requests,
  COUNT(*) FILTER (WHERE e.input_tokens > 0 AND e.cache_usage_reported IS NULL)::bigint AS cache_unknown_requests,
  COALESCE(SUM(e.input_tokens) FILTER (WHERE e.input_tokens > 0 AND e.cache_usage_reported IS TRUE),0)::bigint AS cache_reported_input_tokens,
  COALESCE(SUM(e.cached_tokens) FILTER (WHERE e.input_tokens > 0 AND e.cache_usage_reported IS TRUE),0)::bigint AS cache_reported_tokens";

#[derive(Serialize, FromRow)]
struct CacheCoverage {
    cache_reported_requests: i64,
    cache_unreported_requests: i64,
    cache_unknown_requests: i64,
    cache_reported_input_tokens: i64,
    cache_reported_tokens: i64,
}

#[derive(Serialize, FromRow)]
struct UsageTotals {
    #[sqlx(flatten)]
    #[serde(flatten)]
    cache: CacheCoverage,
    requests: i64,
    successful: i64,
    input_tokens: i64,
    output_tokens: i64,
    cached_tokens: i64,
    active_models: i64,
    avg_duration_ms: Option<f64>,
    avg_ttfb_ms: Option<f64>,
}

#[derive(FromRow)]
pub(crate) struct BucketModelRow {
    bucket: i64,
    #[sqlx(flatten)]
    performance: Performance,
    requests: i64,
    input_tokens: i64,
    output_tokens: i64,
    cost_usd: Option<f64>,
}

#[derive(Serialize)]
pub(crate) struct TrendPoint {
    start: DateTime<Utc>,
    requests: i64,
    tokens: i64,
    /// Priced spend; requests without a cost add nothing.
    estimated_cost_usd: f64,
    #[serde(flatten)]
    performance: Performance,
}

pub(crate) fn build_trend(plan: &BucketPlan, buckets: Vec<BucketModelRow>) -> Vec<TrendPoint> {
    let mut trend = (0..plan.count)
        .map(|index| TrendPoint {
            start: plan.start + Duration::seconds(plan.step_seconds * index),
            requests: 0,
            tokens: 0,
            estimated_cost_usd: 0.0,
            performance: Performance::default(),
        })
        .collect::<Vec<_>>();
    for row in buckets {
        if let Some(point) = usize::try_from(row.bucket)
            .ok()
            .and_then(|index| trend.get_mut(index))
        {
            point.performance = row.performance;
            point.requests += row.requests;
            point.tokens += row.input_tokens + row.output_tokens;
            point.estimated_cost_usd += row.cost_usd.unwrap_or(0.0);
        }
    }
    trend
}

#[derive(Serialize, FromRow)]
pub(crate) struct ModelBreakdown {
    model: String,
    estimated_cost_usd: Option<f64>,
    requests: i64,
    tokens: i64,
}

#[derive(Serialize, FromRow)]
struct ChannelBreakdown {
    #[sqlx(flatten)]
    #[serde(flatten)]
    cache: CacheCoverage,
    channel_id: Option<Uuid>,
    channel_name: Option<String>,
    estimated_cost_usd: Option<f64>,
    requests: i64,
    successful: i64,
    client_errors: i64,
    server_errors: i64,
    input_tokens: i64,
    output_tokens: i64,
    cached_tokens: i64,
    avg_ttfb_ms: Option<f64>,
    avg_duration_ms: Option<f64>,
    #[sqlx(flatten)]
    #[serde(flatten)]
    performance: Performance,
}

#[derive(Serialize, FromRow)]
struct KeyOption {
    id: Uuid,
    name: String,
    revoked: bool,
}

/// Recent raw events for the overview page.
pub async fn recent(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<UsageView>>> {
    let user = auth::current_user(&state, &headers).await?;
    let sql = format!(
        "SELECT {EVENT_COLUMNS} WHERE e.account_id=$1 ORDER BY e.created_at DESC LIMIT 200"
    );
    Ok(Json(
        sqlx::query_as::<_, UsageView>(&sql)
            .bind(user.account_id)
            .fetch_all(&state.db)
            .await?,
    ))
}

#[derive(Serialize, FromRow)]
pub struct UsageDetail {
    #[serde(flatten)]
    #[sqlx(flatten)]
    event: UsageView,
    request_diagnostics: Option<sqlx::types::Json<Diagnostics>>,
}

/// Details are loaded on demand; lists and CSV exports stay lightweight.
pub async fn detail(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<UsageDetail>> {
    let user = auth::current_user(&state, &headers).await?;
    let sql =
        format!("SELECT e.request_diagnostics,{EVENT_COLUMNS} WHERE e.account_id=$1 AND e.id=$2");
    let mut detail = sqlx::query_as::<_, UsageDetail>(&sql)
        .bind(user.account_id)
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::NotFound)?;
    if let Some(diagnostics) = &mut detail.request_diagnostics {
        let ids: Vec<_> = diagnostics.attempts.iter().map(|a| a.channel_id).collect();
        let names: std::collections::HashMap<Uuid, String> =
            sqlx::query_as("SELECT id,name FROM channels WHERE account_id=$1 AND id=ANY($2)")
                .bind(user.account_id)
                .bind(ids)
                .fetch_all(&state.db)
                .await?
                .into_iter()
                .collect();
        for attempt in &mut diagnostics.attempts {
            attempt.channel_name = names.get(&attempt.channel_id).cloned();
        }
    }
    Ok(Json(detail))
}

/// Only successful requests can incur charges, so failed requests without a
/// cost never make an amount unknown.
pub(crate) const UNKNOWN_COST: &str = "e.cost_usd IS NULL AND e.status BETWEEN 200 AND 299";

pub async fn summary(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<UsageQuery>,
) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    let plan = query.plan(&state, user.account_id).await?;
    let filters = query.filters(user.account_id, plan.start);

    let totals_sql = format!(
        "SELECT COUNT(*)::bigint AS requests,
                COUNT(*) FILTER (WHERE e.status BETWEEN 200 AND 299)::bigint AS successful,
                COALESCE(SUM(e.input_tokens),0)::bigint AS input_tokens,
                COALESCE(SUM(e.output_tokens),0)::bigint AS output_tokens,
                COALESCE(SUM(e.cached_tokens),0)::bigint AS cached_tokens,
                {CACHE_COLUMNS},
                COUNT(DISTINCT e.model)::bigint AS active_models,
                AVG(e.duration_ms)::double precision AS avg_duration_ms,
                AVG(e.ttfb_ms)::double precision AS avg_ttfb_ms
         FROM usage_events e WHERE {FILTER}"
    );
    let totals = bind_filters!(sqlx::query_as::<_, UsageTotals>(&totals_sql), filters)
        .fetch_one(&state.db)
        .await?;

    let trend_sql = format!(
        "SELECT FLOOR(EXTRACT(EPOCH FROM (e.created_at - $2))::double precision / $9::double precision)::bigint AS bucket,
                SUM(e.cost_usd)::float8 AS cost_usd,
                COUNT(*)::bigint AS requests,
                COALESCE(SUM(e.input_tokens),0)::bigint AS input_tokens,
                COALESCE(SUM(e.output_tokens),0)::bigint AS output_tokens,
                {PERFORMANCE_COLUMNS}
         FROM usage_events e WHERE {FILTER}
         GROUP BY bucket"
    );
    let buckets = bind_filters!(sqlx::query_as::<_, BucketModelRow>(&trend_sql), filters)
        .bind(plan.step_seconds)
        .fetch_all(&state.db)
        .await?;
    let trend = build_trend(&plan, buckets);

    let models_sql = format!(
        "SELECT e.model, COUNT(*)::bigint AS requests,
                SUM(e.cost_usd)::float8 AS estimated_cost_usd,
                COALESCE(SUM(COALESCE(e.input_tokens,0)+COALESCE(e.output_tokens,0)),0)::bigint AS tokens
         FROM usage_events e WHERE {FILTER}
         GROUP BY e.model ORDER BY requests DESC, e.model LIMIT 50"
    );
    let models = bind_filters!(sqlx::query_as::<_, ModelBreakdown>(&models_sql), filters)
        .fetch_all(&state.db)
        .await?;

    let channels_sql = format!(
        "SELECT e.channel_id, c.name AS channel_name, COUNT(*)::bigint AS requests,
                SUM(e.cost_usd)::float8 AS estimated_cost_usd,
                COUNT(*) FILTER (WHERE e.status BETWEEN 200 AND 299)::bigint AS successful,
                COUNT(*) FILTER (WHERE e.status BETWEEN 400 AND 499)::bigint AS client_errors,
                COUNT(*) FILTER (WHERE e.status >= 500)::bigint AS server_errors,
                COALESCE(SUM(e.input_tokens),0)::bigint AS input_tokens,
                COALESCE(SUM(e.output_tokens),0)::bigint AS output_tokens,
                COALESCE(SUM(e.cached_tokens),0)::bigint AS cached_tokens,
                {CACHE_COLUMNS},
                AVG(e.ttfb_ms)::double precision AS avg_ttfb_ms,
                AVG(e.duration_ms)::double precision AS avg_duration_ms,
                {PERFORMANCE_COLUMNS}
         FROM usage_events e
         LEFT JOIN channels c ON c.id=e.channel_id AND c.account_id=e.account_id
         WHERE {FILTER}
         GROUP BY e.channel_id, c.name ORDER BY requests DESC LIMIT 50"
    );
    let channels = bind_filters!(
        sqlx::query_as::<_, ChannelBreakdown>(&channels_sql),
        filters
    )
    .fetch_all(&state.db)
    .await?;

    // Model filter options ignore the other filters so picking one model does
    // not hide the rest from the dropdown.
    let model_options: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT model FROM usage_events
         WHERE account_id=$1 AND created_at >= $2 ORDER BY model LIMIT 200",
    )
    .bind(user.account_id)
    .bind(plan.start)
    .fetch_all(&state.db)
    .await?;
    // Revoked keys stay selectable while they still have usage in range.
    let key_options = sqlx::query_as::<_, KeyOption>(
        "SELECT k.id, k.name, (k.revoked_at IS NOT NULL) AS revoked FROM virtual_api_keys k
         WHERE k.account_id=$1 AND (k.revoked_at IS NULL OR EXISTS(
           SELECT 1 FROM usage_events e WHERE e.virtual_key_id=k.id AND e.account_id=$1 AND e.created_at >= $2))
         ORDER BY revoked, k.name LIMIT 200",
    )
    .bind(user.account_id)
    .bind(plan.start)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(json!({
        "since": plan.start,
        "bucket_seconds": plan.step_seconds,
        "totals": totals,
        "trend": trend,
        "models": models,
        "channels": channels,
        "model_options": model_options,
        "key_options": key_options,
    })))
}

pub async fn events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<UsageQuery>,
) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    let filters = query.filters(
        user.account_id,
        query.plan(&state, user.account_id).await?.start,
    );
    // Large limits are only used by CSV export; the table pages by 10.
    let limit = query.limit.unwrap_or(10).clamp(1, 5000);
    let count_sql = format!("SELECT COUNT(*) FROM usage_events e WHERE {FILTER}");
    let total = bind_filters!(sqlx::query_scalar::<_, i64>(&count_sql), filters)
        .fetch_one(&state.db)
        .await?;
    let page = query
        .page
        .unwrap_or(1)
        .clamp(1, ((total - 1) / limit + 1).max(1));
    let rows_sql = format!(
        "SELECT {EVENT_COLUMNS} WHERE {FILTER}
         ORDER BY e.created_at DESC, e.id DESC LIMIT $9 OFFSET $10"
    );
    let rows = bind_filters!(sqlx::query_as::<_, UsageView>(&rows_sql), filters)
        .bind(limit)
        .bind((page - 1) * limit)
        .fetch_all(&state.db)
        .await?;
    Ok(Json(
        json!({"items": rows, "total": total, "page": page, "page_size": limit}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn snapshot_totals_count_priced_spend_and_keep_empty_buckets() {
        let plan = bucket_plan(UsageRange::Week, Utc::now(), 480);
        let row = |bucket, cost| BucketModelRow {
            bucket,
            requests: 1,
            input_tokens: 100,
            output_tokens: 20,
            cost_usd: cost,
            performance: Performance::default(),
        };
        let trend = build_trend(
            &plan,
            vec![
                row(1, Some(2.5)),
                row(1, Some(1.0)),
                row(2, Some(0.0)),
                row(3, None),
                row(4, Some(2.0)),
                row(4, None),
            ],
        );
        assert_eq!(trend[0].estimated_cost_usd, 0.0);
        assert_eq!(trend[1].estimated_cost_usd, 3.5);
        assert_eq!(trend[1].tokens, 240);
        assert_eq!(trend[2].estimated_cost_usd, 0.0);
        assert_eq!(trend[3].estimated_cost_usd, 0.0);
        assert_eq!(trend[4].estimated_cost_usd, 2.0);
        assert_eq!(trend[4].requests, 2);
    }

    #[test]
    fn day_buckets_end_at_current_local_hour() {
        let now = Utc.with_ymd_and_hms(2026, 10, 3, 18, 42, 10).unwrap();
        let plan = bucket_plan(UsageRange::Day, now, 480);
        assert_eq!(plan.count, 24);
        assert_eq!(plan.step_seconds, 3600);
        // 02:42 local on Oct 4 -> last bucket 02:00 local -> first 03:00 local Oct 3.
        assert_eq!(
            plan.start,
            Utc.with_ymd_and_hms(2026, 10, 2, 19, 0, 0).unwrap()
        );
    }

    #[test]
    fn week_buckets_align_to_local_midnight() {
        let now = Utc.with_ymd_and_hms(2026, 10, 3, 18, 42, 10).unwrap();
        let plan = bucket_plan(UsageRange::Week, now, 480);
        assert_eq!(plan.count, 7);
        // Local today is Oct 4, so the window starts at local midnight Sep 28.
        assert_eq!(
            plan.start,
            Utc.with_ymd_and_hms(2026, 9, 27, 16, 0, 0).unwrap()
        );
        let utc = bucket_plan(UsageRange::Month, now, 0);
        assert_eq!(
            utc.start,
            Utc.with_ymd_and_hms(2026, 9, 4, 0, 0, 0).unwrap()
        );
    }

    #[test]
    fn offset_is_clamped() {
        let now = Utc.with_ymd_and_hms(2026, 10, 3, 0, 0, 0).unwrap();
        assert_eq!(
            bucket_plan(UsageRange::Week, now, 100_000).start,
            bucket_plan(UsageRange::Week, now, 840).start
        );
    }

    #[test]
    fn all_time_buckets_cover_the_first_request_and_widen_with_span() {
        // Local (UTC+8) today is Oct 4; buckets end at local midnight Oct 5.
        let now = Utc.with_ymd_and_hms(2026, 10, 3, 18, 42, 10).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 10, 4, 16, 0, 0).unwrap();
        for (first, step_days, count) in [
            (now, 1, 1),
            (Utc.with_ymd_and_hms(2026, 8, 6, 3, 0, 0).unwrap(), 1, 60),
            (Utc.with_ymd_and_hms(2026, 8, 4, 3, 0, 0).unwrap(), 7, 9),
            // 2024-01-01 to 2026-10-04 is 1008 local days.
            (Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(), 30, 34),
        ] {
            let plan = all_time_plan(first, now, 480);
            assert_eq!(plan.step_seconds, step_days * 86_400);
            assert_eq!(plan.count, count);
            assert!(plan.start <= first);
            assert_eq!(
                plan.start + Duration::seconds(plan.step_seconds * plan.count),
                end
            );
        }
        // Without any request the range is just today.
        assert_eq!(bucket_plan(UsageRange::All, now, 480).count, 1);
    }
}
