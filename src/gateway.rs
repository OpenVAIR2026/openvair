use crate::edition::PRODUCT;
use crate::{
    auth::hash_token,
    error::{AppError, Result},
    model_rules, models, pricing,
    request_diagnostics::{Outcome, Timings, elapsed_ms},
    state::AppState,
};
use axum::{
    Json,
    body::{Body, to_bytes},
    extract::State,
    http::{HeaderMap, HeaderName, HeaderValue, Request, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use futures_util::{FutureExt, StreamExt, stream};
use serde_json::{Value, json};
use sqlx::{Row, postgres::PgRow};
use std::net::IpAddr;
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    time::Instant,
};
use tokio::sync::OwnedSemaphorePermit;
use uuid::Uuid;

pub(crate) const UPSTREAM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// Largest non-streamed upstream answer relayed to a client; images in
/// base64 make these far larger than text alone.
pub(crate) const MAX_UPSTREAM_BODY: usize = 64 * 1024 * 1024;
/// Largest model list, probe or price document read from an upstream.
pub(crate) const MAX_METADATA_BODY: usize = 16 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BodyError {
    TooLarge,
    Read,
}

/// Reads a response body, refusing one larger than `limit` bytes before
/// buffering it.
pub(crate) async fn read_limited(
    response: reqwest::Response,
    limit: usize,
) -> std::result::Result<axum::body::Bytes, BodyError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(BodyError::TooLarge);
    }
    collect_limited(response.bytes_stream(), limit).await
}

/// Reads a JSON body of at most `limit` bytes.
pub(crate) async fn read_json_limited<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    limit: usize,
) -> std::result::Result<T, String> {
    let body = read_limited(response, limit)
        .await
        .map_err(|error| match error {
            BodyError::TooLarge => "response too large".to_owned(),
            BodyError::Read => "failed to read response".to_owned(),
        })?;
    serde_json::from_slice(&body).map_err(|error| error.to_string())
}

async fn collect_limited<S, E>(
    mut stream: S,
    limit: usize,
) -> std::result::Result<axum::body::Bytes, BodyError>
where
    S: futures_util::Stream<Item = std::result::Result<axum::body::Bytes, E>> + Unpin,
{
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| BodyError::Read)?;
        if body.len() + chunk.len() > limit {
            return Err(BodyError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body.into())
}

#[cfg(test)]
#[path = "gateway_db_tests.rs"]
mod db_tests;

#[derive(Clone)]
struct GatewayKey {
    id: Uuid,
    account_id: Uuid,
    allowed_models: Vec<String>,
    model_mode: String,
    requests_per_minute: Option<i32>,
    token_limit_daily: Option<i64>,
    cost_limit_daily_usd: Option<f64>,
    cost_limit_total_usd: Option<f64>,
    concurrency_limit: i32,
    usage_reset_daily_at: Option<DateTime<Utc>>,
    usage_reset_total_at: Option<DateTime<Utc>>,
    /// `None` routes to every channel; otherwise only to these channels.
    channel_ids: Option<Vec<Uuid>>,
}

#[derive(Clone)]
struct Candidate {
    channel_id: Uuid,
    base_url: String,
    protocol: String,
    upstream_model: String,
    secret: String,
    health: ChannelHealth,
    cooldown: crate::routing::Cooldown,
    /// Requests in flight through the channel at once; `None` means no cap.
    concurrency_limit: Option<i32>,
}

/// A channel's availability state and the routing settings that go with it.
#[derive(Clone, Copy)]
struct ChannelHealth {
    /// Judged unavailable under the exclude policy: tried only after every
    /// other channel, so a model whose channels all failed is still served.
    excluded: bool,
    unhealthy: bool,
    failures: i32,
    /// Real requests count toward availability only while it is monitored.
    monitored: bool,
    first_token_timeout: std::time::Duration,
}

/// Columns `ChannelHealth::from_row` reads, for queries over `channels c`
/// joined to `channel_health h` and `channel_monitor_configs mc`.
const HEALTH_COLUMNS: &str = "COALESCE(h.status='unhealthy',false) AS unhealthy, \
     COALESCE(h.status='unhealthy' AND mc.availability_route_policy IS DISTINCT FROM 'observe',false) AS availability_excluded, \
     COALESCE(h.consecutive_failures,0) AS health_failures, \
     COALESCE(mc.availability_enabled,c.kind='upstream') AS availability_monitored, \
     c.first_token_timeout_seconds";

impl ChannelHealth {
    fn from_row(row: &PgRow) -> Result<Self> {
        let invalid = |_| AppError::Internal("invalid channel health".into());
        let seconds: i32 = row
            .try_get("first_token_timeout_seconds")
            .map_err(invalid)?;
        Ok(Self {
            excluded: row.try_get("availability_excluded").map_err(invalid)?,
            unhealthy: row.try_get("unhealthy").map_err(invalid)?,
            failures: row.try_get("health_failures").map_err(invalid)?,
            monitored: row.try_get("availability_monitored").map_err(invalid)?,
            first_token_timeout: std::time::Duration::from_secs(seconds.max(1) as u64),
        })
    }
}

/// Largest gateway request body; Caddy allows slightly more so this error is the one clients see.
const MAX_REQUEST_BODY_BYTES: usize = 50 * 1024 * 1024;

/// Columns `GatewayKey::from_row` reads, for queries over `virtual_api_keys v`.
const KEY_COLUMNS: &str = "v.id, v.account_id, v.allowed_models, v.model_mode, v.requests_per_minute, v.token_limit_daily, v.cost_limit_daily_usd, v.cost_limit_total_usd, v.concurrency_limit, v.usage_reset_daily_at, v.usage_reset_total_at, \
     CASE WHEN v.channel_mode='selected' THEN ARRAY(SELECT vc.channel_id FROM virtual_key_channels vc WHERE vc.virtual_key_id=v.id) END AS channel_ids";

impl GatewayKey {
    fn from_row(row: &PgRow) -> Result<Self> {
        let invalid = |_| AppError::Internal("invalid virtual key row".into());
        let allowed_models: Vec<String> = serde_json::from_value(
            row.try_get::<Value, _>("allowed_models")
                .unwrap_or_else(|_| json!([])),
        )
        .unwrap_or_default();
        Ok(Self {
            id: row.try_get("id").map_err(invalid)?,
            account_id: row.try_get("account_id").map_err(invalid)?,
            allowed_models,
            model_mode: row.try_get("model_mode")?,
            requests_per_minute: row.try_get("requests_per_minute").ok(),
            token_limit_daily: row.try_get("token_limit_daily").ok(),
            cost_limit_daily_usd: row.try_get("cost_limit_daily_usd").ok(),
            cost_limit_total_usd: row.try_get("cost_limit_total_usd").ok(),
            concurrency_limit: row.try_get("concurrency_limit").unwrap_or(4),
            usage_reset_daily_at: row.try_get("usage_reset_daily_at")?,
            usage_reset_total_at: row.try_get("usage_reset_total_at")?,
            channel_ids: row.try_get("channel_ids")?,
        })
    }

    fn allows_model(&self, model: &str) -> bool {
        self.model_mode == "all" || model_rules::allows(&self.allowed_models, model)
    }

    fn allows_channel(&self, channel_id: Uuid) -> bool {
        self.channel_ids
            .as_ref()
            .is_none_or(|selected| selected.contains(&channel_id))
    }
}

/// A channel backed by another account's virtual key. Requests through it are
/// dispatched internally with the source key's permissions and limits.
struct RelayChannel {
    channel_id: Uuid,
    priority: i32,
    model_mapping: serde_json::Map<String, Value>,
    source: GatewayKey,
    health: ChannelHealth,
}

enum Route {
    Upstream(Candidate),
    Relay {
        channel_id: Uuid,
        model: String,
        source: GatewayKey,
        health: ChannelHealth,
    },
}

/// Most VAIR key channels a request passes through in a row. The account the
/// last one reaches routes only to its upstream channels.
const MAX_RELAY_HOPS: usize = 5;
/// Most VAIR key channels one request expands across all its routes, so
/// branches that fail at every layer cannot multiply the attempts.
const MAX_RELAY_EXPANSIONS: usize = 16;

const RELAY_UNAVAILABLE: (StatusCode, &str) = (
    StatusCode::NOT_FOUND,
    "model is not available through this VAIR key channel",
);

/// Source-key state held for one hop of a relayed request while it is in flight.
struct Relay {
    // The channel one level up that reached this key.
    channel_id: Uuid,
    source: GatewayKey,
    model: String,
    permit: OwnedSemaphorePermit,
}

/// A hop being routed: its routes not yet tried and, once something was sent
/// through it, the channel of its last attempt with that channel's failure
/// event when it relayed further.
struct RelayFrame {
    relay: Relay,
    routes: std::vec::IntoIter<Route>,
    last: Option<(Uuid, Option<Uuid>)>,
}

/// The hops a request is relayed through, outermost first. A VAIR key channel
/// is expanded only when routing reaches it, so a branch that is never needed
/// consumes none of its key's limits. A path visits each account at most once.
struct RelayChain {
    // The caller's account, where every path starts.
    origin: Uuid,
    frames: Vec<RelayFrame>,
    expansions: usize,
}

enum RelayStep {
    Candidate(Candidate),
    // A nested VAIR key channel could not be used; nothing was sent through it.
    Unavailable(StatusCode, &'static str),
    // The innermost hop ran out of routes and was removed.
    Exhausted(RelayFrame),
    Done,
}

impl RelayChain {
    fn new(origin: Uuid) -> Self {
        Self {
            origin,
            frames: Vec::new(),
            expansions: 0,
        }
    }

    fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Applies a VAIR key channel's source key and pushes it as the innermost hop.
    async fn enter(
        &mut self,
        state: &AppState,
        protocol: &str,
        channel_id: Uuid,
        source: GatewayKey,
        model: String,
    ) -> Result<std::result::Result<(), (StatusCode, &'static str)>> {
        if self.frames.len() >= MAX_RELAY_HOPS || self.expansions >= MAX_RELAY_EXPANSIONS {
            return Ok(Err(RELAY_UNAVAILABLE));
        }
        self.expansions += 1;
        let mut path: Vec<Uuid> = std::iter::once(self.origin)
            .chain(
                self.frames
                    .iter()
                    .map(|frame| frame.relay.source.account_id),
            )
            .collect();
        path.push(source.account_id);
        // The account at the last hop routes only to its upstream channels.
        let path = (path.len() <= MAX_RELAY_HOPS).then_some(path.as_slice());
        Ok(
            prepare_relay(state, protocol, channel_id, source, model, path)
                .await?
                .map(|(routes, relay)| {
                    self.frames.push(RelayFrame {
                        relay,
                        routes: routes.into_iter(),
                        last: None,
                    })
                }),
        )
    }

    /// The innermost hop's next upstream, expanding VAIR key channels on the way.
    async fn next(&mut self, state: &AppState, protocol: &str) -> Result<RelayStep> {
        loop {
            let Some(frame) = self.frames.last_mut() else {
                return Ok(RelayStep::Done);
            };
            match frame.routes.next() {
                Some(Route::Upstream(candidate)) => return Ok(RelayStep::Candidate(candidate)),
                Some(Route::Relay {
                    channel_id,
                    model,
                    source,
                    ..
                }) => {
                    if let Err((status, message)) = self
                        .enter(state, protocol, channel_id, source, model)
                        .await?
                    {
                        return Ok(RelayStep::Unavailable(status, message));
                    }
                }
                None => {
                    return Ok(self
                        .frames
                        .pop()
                        .map_or(RelayStep::Done, RelayStep::Exhausted));
                }
            }
        }
    }

    /// Notes an attempt at an upstream channel of the innermost hop.
    fn sent(&mut self, upstream_channel: Uuid) {
        if let Some(frame) = self.frames.last_mut() {
            frame.last = Some((upstream_channel, None));
        }
    }

    /// Hands a removed hop's failure to its parent, whose last attempt was the
    /// channel that reached it.
    fn failed(&mut self, frame: &RelayFrame, event: Option<Uuid>) {
        if frame.last.is_some()
            && let Some(parent) = self.frames.last_mut()
        {
            parent.last = Some((frame.relay.channel_id, event));
        }
    }

    fn has_routes(&self) -> bool {
        self.frames
            .iter()
            .any(|frame| !frame.routes.as_slice().is_empty())
    }

    /// Drops the routes left so the hops unwind and record their failures.
    fn abandon(&mut self) {
        for frame in &mut self.frames {
            frame.routes = Vec::new().into_iter();
        }
    }

    /// Each hop with the channel in its account that leads to `upstream_channel`,
    /// outermost first.
    fn served(&self, upstream_channel: Uuid) -> Vec<(&Relay, Uuid)> {
        self.frames
            .iter()
            .enumerate()
            .map(|(index, frame)| {
                let next = self.frames.get(index + 1);
                (
                    &frame.relay,
                    next.map_or(upstream_channel, |next| next.relay.channel_id),
                )
            })
            .collect()
    }

    fn take_permits(&mut self) -> impl Iterator<Item = OwnedSemaphorePermit> + use<> {
        std::mem::take(&mut self.frames)
            .into_iter()
            .map(|frame| frame.relay.permit)
    }
}

pub async fn handle(State(state): State<AppState>, request: Request<Body>) -> Response {
    let origin = request_origin(request.headers());
    match handle_inner(state, request).await {
        Ok(response) => response,
        Err(error) => with_help(error, origin.as_deref()).into_response(),
    }
}

/// The site origin the client used, so key errors can link to the key page.
fn request_origin(headers: &HeaderMap) -> Option<String> {
    let host = headers.get(header::HOST)?.to_str().ok()?;
    if host.is_empty()
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b))
    {
        return None;
    }
    let scheme = match headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
    {
        Some("http") => "http",
        Some(_) => "https",
        None if host.starts_with("localhost") || host.starts_with("127.") => "http",
        None => "https",
    };
    Some(format!("{scheme}://{host}"))
}

/// Points the holder of a usable but restricted key at its self-service page.
fn with_help(error: AppError, origin: Option<&str>) -> AppError {
    match (error, origin) {
        (AppError::Coded(status, code, message), Some(origin)) if code != "invalid_api_key" => {
            AppError::Coded(
                status,
                code,
                format!(
                    "{message}。在 {origin}{} 查看此 Key 的状态与额度",
                    crate::key_holder::PAGE_PATH
                ),
            )
        }
        (error, _) => error,
    }
}

/// Names the rate limit a client request hit.
fn limit_error(error: AppError, code: &'static str, message: &str) -> AppError {
    match error {
        AppError::RateLimited => {
            AppError::Coded(StatusCode::TOO_MANY_REQUESTS, code, message.to_owned())
        }
        error => error,
    }
}

const DAILY_TOKENS_USED: &str = "此 Key 今日的 Token 额度已用完，次日自动恢复，或请发放方重置";
const DAILY_COST_USED: &str = "此 Key 今日的费用额度已用完，次日自动恢复，或请发放方重置";
const TOTAL_COST_USED: &str = "此 Key 的总费用额度已用完，请发放方提高上限或重置";
/// A key's spend and token use inside its daily and lifetime windows, in one
/// pass over `usage_key_limits_idx`. The day starts in the database's time
/// zone. `$5` says whether the lifetime window is wanted at all, so a key
/// without a total limit only reads today's rows.
const KEY_USAGE_SQL: &str = "WITH bounds AS (
   SELECT GREATEST(date_trunc('day',now()),COALESCE($3,'-infinity'::timestamptz)) AS daily_from,
          CASE WHEN $5 THEN COALESCE($4,'-infinity'::timestamptz) END AS total_from)
 SELECT COALESCE(SUM(COALESCE(e.input_tokens,0)+COALESCE(e.output_tokens,0))
          FILTER (WHERE e.created_at>=w.daily_from),0)::bigint AS tokens_daily,
        COALESCE(SUM(e.cost_usd) FILTER (WHERE e.created_at>=w.daily_from),0)::float8 AS cost_daily,
        COALESCE(SUM(e.cost_usd) FILTER (WHERE e.created_at>=w.total_from),0)::float8 AS cost_total
 FROM bounds w LEFT JOIN usage_events e ON e.account_id=$1 AND e.virtual_key_id=$2
   AND e.created_at>=LEAST(w.daily_from,COALESCE(w.total_from,w.daily_from))";

/// Checks the key's daily token, daily cost and total cost limits with one
/// query.
async fn enforce_limits(state: &AppState, key: &GatewayKey) -> Result<()> {
    if key.token_limit_daily.is_none()
        && key.cost_limit_daily_usd.is_none()
        && key.cost_limit_total_usd.is_none()
    {
        return Ok(());
    }
    let row = sqlx::query(KEY_USAGE_SQL)
        .bind(key.account_id)
        .bind(key.id)
        .bind(key.usage_reset_daily_at)
        .bind(key.usage_reset_total_at)
        .bind(key.cost_limit_total_usd.is_some())
        .fetch_one(&state.db)
        .await?;
    let reached = |code, message: &str| {
        Err(AppError::Coded(
            StatusCode::TOO_MANY_REQUESTS,
            code,
            message.to_owned(),
        ))
    };
    if let Some(limit) = key.token_limit_daily
        && row.try_get::<i64, _>("tokens_daily")? >= limit
    {
        return reached("daily_token_limit", DAILY_TOKENS_USED);
    }
    if let Some(limit) = key.cost_limit_daily_usd
        && row.try_get::<f64, _>("cost_daily")? >= limit
    {
        return reached("daily_cost_limit", DAILY_COST_USED);
    }
    if let Some(limit) = key.cost_limit_total_usd
        && row.try_get::<f64, _>("cost_total")? >= limit
    {
        return reached("total_cost_limit", TOTAL_COST_USED);
    }
    Ok(())
}

async fn handle_inner(state: AppState, request: Request<Body>) -> Result<Response> {
    let started = Instant::now();
    let path = request.uri().path().to_owned();
    let query = request.uri().query().map(str::to_owned);
    let method = request.method().clone();
    let headers = request.headers().clone();
    let key = authenticate_virtual_key(&state, &headers).await?;
    if method == axum::http::Method::GET && (path == "/v1/models" || path == "/v1beta/models") {
        return model_catalog(&state, &key).await;
    }
    if method != axum::http::Method::POST {
        return Err(AppError::BadRequest(
            "gateway endpoints accept POST, except GET /v1/models".into(),
        ));
    }
    let body = to_bytes(request.into_body(), MAX_REQUEST_BODY_BYTES)
        .await
        .map_err(|_| AppError::PayloadTooLarge("request body exceeds 50 MiB".into()))?;
    let mut payload: Value = if body.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&body)
            .map_err(|_| AppError::BadRequest("request body must be valid JSON".into()))?
    };
    let (protocol, public_model) = request_protocol_model(&path, &payload)?;
    if !model_rules::concrete(&public_model) {
        return Err(AppError::BadRequest("请求必须提供具体模型 ID".into()));
    }
    if !key.allows_model(&public_model) {
        return Err(AppError::Coded(
            StatusCode::FORBIDDEN,
            "model_not_allowed",
            format!("此 Key 无权使用模型 {public_model}"),
        ));
    }
    state
        .redis_rate_limit(key.id, key.requests_per_minute)
        .await
        .map_err(|error| {
            let rpm = key.requests_per_minute.unwrap_or_default();
            let message = format!("已达到此 Key 每分钟 {rpm} 次请求的上限，请稍后重试");
            limit_error(error, "rpm_limit", &message)
        })?;
    let semaphore = state
        .semaphore(key.id, key.concurrency_limit.max(1) as usize)
        .await;
    let queue_started = Instant::now();
    let permit = match semaphore.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            // A request already over its limits fails at once instead of queueing.
            enforce_limits(&state, &key).await?;
            semaphore
                .acquire_owned()
                .await
                .map_err(|_| AppError::Internal("concurrency limiter closed".into()))?
        }
    };
    let queue_ms = elapsed_ms(queue_started);
    let mut timings = Timings::new(started, key.account_id, queue_ms);
    // Checked while holding the permit, so requests racing past a limit are
    // bounded by the key's concurrency instead of by how many arrive at once.
    enforce_limits(&state, &key).await?;
    let routes = load_routes(
        &state,
        &key,
        &protocol,
        &public_model,
        Some(&[key.account_id]),
    )
    .await?;
    if routes.is_empty() {
        return Err(AppError::Coded(
            StatusCode::NOT_FOUND,
            "model_unavailable",
            format!("此 Key 当前没有可以调用模型 {public_model} 的渠道"),
        ));
    }
    let mut last_status = StatusCode::BAD_GATEWAY;
    let mut last_body = Vec::new();
    let mut attempted = HashSet::new();
    // The caller's channel that produced the final status and, when it relayed
    // through another account's key, that key's usage event.
    let mut last_channel = None;
    let mut last_relayed_event = None;
    let prices = state.prices().await?;
    let mut last_upstream_model = None;
    let mut last_price = None;
    // Candidates that cannot be sent are skipped; this error only surfaces when
    // no upstream was attempted, so an earlier upstream failure is not hidden.
    let mut skipped = None;
    let mut permit = Some(permit);
    // Gemini selects streaming by endpoint rather than a body field.
    let is_stream = payload
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || (protocol == "gemini" && path.ends_with(":streamGenerateContent"));
    // Counted before any field is added, so it reflects what the client sent.
    let input_chars = crate::metering::count_input_chars(&payload);
    let input_shapes = crate::token_estimate::spawn(
        &path,
        &payload,
        routes.iter().map(|route| match route {
            Route::Upstream(candidate) => candidate.upstream_model.as_str(),
            Route::Relay { model, .. } => model.as_str(),
        }),
    );
    timings.acp_stripped(crate::bili_tags::strip(&path, &mut payload));
    // Only Chat Completions has `stream_options.include_usage`; the Responses
    // API always reports usage in `response.completed` and rejects the field.
    if is_stream && path == "/v1/chat/completions" {
        if !payload.get("stream_options").is_some_and(Value::is_object) {
            payload["stream_options"] = json!({});
        }
        // Usage is required for cost accounting, even if the client opted out.
        if let Some(options) = payload
            .get_mut("stream_options")
            .and_then(Value::as_object_mut)
        {
            options.insert("include_usage".into(), Value::Bool(true));
        }
    }
    let route_count = routes.len();
    let mut chain = RelayChain::new(key.account_id);
    'routes: for (route_index, route) in routes.into_iter().enumerate() {
        let route_started = Instant::now();
        let (channel_id, health, mut direct) = match route {
            Route::Upstream(candidate) => (candidate.channel_id, candidate.health, Some(candidate)),
            Route::Relay {
                channel_id,
                model,
                source,
                health,
            } => match chain
                .enter(&state, &protocol, channel_id, source, model)
                .await?
            {
                Ok(()) => (channel_id, health, None),
                Err((status, message)) => {
                    timings.unavailable(channel_id, route_started, status.as_u16());
                    last_channel = Some(channel_id);
                    last_relayed_event = None;
                    last_status = status;
                    last_body = json!({"error": {"message": message}})
                        .to_string()
                        .into_bytes();
                    continue;
                }
            },
        };
        let mut sent = false;
        let mut stop = false;
        // Why this route's last attempt counts against the channel's availability.
        let mut route_failure = None;
        loop {
            let candidate = match direct.take() {
                Some(candidate) => candidate,
                None => match chain.next(&state, &protocol).await? {
                    RelayStep::Candidate(candidate) => candidate,
                    RelayStep::Unavailable(status, message) => {
                        // Explains the failure only when nothing went out through this channel.
                        if !sent {
                            last_channel = Some(channel_id);
                            last_relayed_event = None;
                            last_status = status;
                            last_body = json!({"error": {"message": message}})
                                .to_string()
                                .into_bytes();
                        }
                        continue;
                    }
                    // The source key's owner sees relayed failures on their own key.
                    RelayStep::Exhausted(frame) => {
                        let event = record_relay_failure(
                            &state,
                            &frame,
                            UsageContext {
                                public_model: &public_model,
                                protocol: &protocol,
                                upstream_model: last_upstream_model.as_deref(),
                                price: last_price,
                                in_flight: false,
                                relayed_event_id: None,
                                source: "gateway",
                                stream: is_stream,
                                input_chars,
                                input_shapes: input_shapes.as_ref(),
                            },
                            last_status.as_u16(),
                            elapsed_ms(started),
                        )
                        .await;
                        if let Some(event) = event {
                            timings
                                .save(&state, &[(event, frame.relay.source.account_id)])
                                .await;
                        }
                        if frame.last.is_some() && chain.is_empty() {
                            last_relayed_event = event;
                        }
                        chain.failed(&frame, event);
                        continue;
                    }
                    RelayStep::Done => break,
                },
            };
            let has_next = chain.has_routes() || route_index + 1 < route_count;
            let identity = format!(
                "{}:{}:{}",
                candidate.channel_id, candidate.upstream_model, candidate.secret
            );
            if !attempted.insert(identity) {
                continue;
            }
            let price = pricing::lookup(&prices, &candidate.upstream_model);
            if !model_rules::concrete(&candidate.upstream_model) {
                skipped.get_or_insert_with(|| {
                    AppError::BadRequest("渠道映射必须指向具体上游模型".into())
                });
                continue;
            }
            let hops = |chain: &RelayChain| {
                chain
                    .served(candidate.channel_id)
                    .iter()
                    .map(|(relay, channel)| (relay.source.account_id, *channel))
                    .collect()
            };
            // A full channel is passed over while another remains; the last
            // one is waited for up to its first token timeout.
            let channel_permit = match candidate.concurrency_limit {
                None => None,
                Some(limit) => {
                    let semaphore = state
                        .semaphore(candidate.channel_id, limit.max(1) as usize)
                        .await;
                    match std::sync::Arc::clone(&semaphore).try_acquire_owned() {
                        Ok(permit) => Some(permit),
                        Err(_) => {
                            let waited = Instant::now();
                            let wait = health.first_token_timeout;
                            let acquired = if has_next {
                                None
                            } else {
                                tokio::time::timeout(wait, semaphore.acquire_owned())
                                    .await
                                    .ok()
                                    .and_then(|permit| permit.ok())
                            };
                            if acquired.is_none() {
                                timings.full(channel_id, hops(&chain), waited);
                                if !has_next {
                                    last_channel = Some(channel_id);
                                    last_relayed_event = None;
                                    last_status = StatusCode::SERVICE_UNAVAILABLE;
                                    last_body = json!({"error": {
                                        "type": "channel_full",
                                        "message": format!(
                                            "渠道并发已满（上限 {limit}），等待 {} 秒仍无空闲名额，请稍后重试",
                                            wait.as_secs()
                                        ),
                                    }})
                                    .to_string()
                                    .into_bytes();
                                }
                                continue;
                            }
                            acquired
                        }
                    }
                }
            };
            last_channel = Some(channel_id);
            last_relayed_event = None;
            sent = true;
            chain.sent(candidate.channel_id);
            let relayed = chain.served(candidate.channel_id);
            last_upstream_model = Some(candidate.upstream_model.clone());
            last_price = price;
            rewrite_model(&mut payload, &protocol, &candidate.upstream_model);
            let upstream_path =
                upstream_path(&path, &protocol, &public_model, &candidate.upstream_model);
            let target = append_client_query(
                &join_base_url(&candidate.base_url, &upstream_path)?,
                query.as_deref(),
            );
            ensure_safe_target(&target).await?;
            let mut builder = state.http.post(target).body(
                serde_json::to_vec(&payload)
                    .map_err(|_| AppError::BadRequest("request could not be serialized".into()))?,
            );
            for (name, value) in &headers {
                if forwards_client_header(name) {
                    builder = builder.header(name, value);
                }
            }
            match candidate.protocol.as_str() {
                "anthropic" => {
                    builder = builder.header("x-api-key", &candidate.secret);
                }
                "gemini" => {
                    builder = builder.header("x-goog-api-key", &candidate.secret);
                }
                _ => {
                    builder = builder.header(
                        header::AUTHORIZATION,
                        format!("Bearer {}", candidate.secret),
                    );
                }
            }
            timings.begin(channel_id, hops(&chain));
            // A stream with no output in time fails over while another channel
            // remains; the last one waits for the upstream timeout instead.
            let first_token_deadline = (is_stream && has_next)
                .then(|| tokio::time::Instant::now() + health.first_token_timeout);
            let first_token_missed = format!(
                "上游 {} 秒内没有输出（首字超时）",
                health.first_token_timeout.as_secs()
            );
            let sent = match first_token_deadline {
                Some(deadline) => tokio::time::timeout_at(deadline, builder.send()).await,
                None => Ok(builder.send().await),
            };
            let response = match sent {
                Ok(Ok(value)) => value,
                Ok(Err(error)) => {
                    let reason = if error.is_timeout() {
                        timings.end_attempt(Outcome::Timeout);
                        format!("上游 {} 秒内未完成（超时）", UPSTREAM_TIMEOUT.as_secs())
                    } else {
                        timings.end_attempt(Outcome::ConnectionError);
                        "无法连接上游或发送请求失败".to_owned()
                    };
                    route_failure = Some((reason, None));
                    last_status = StatusCode::BAD_GATEWAY;
                    continue;
                }
                Err(_) => {
                    timings.end_attempt(Outcome::FirstTokenTimeout);
                    route_failure = Some((first_token_missed, None));
                    last_status = StatusCode::GATEWAY_TIMEOUT;
                    continue;
                }
            };
            let status =
                StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            timings.headers(status.as_u16());
            if !status.is_success() {
                last_status = status;
                last_body.clear();
                let retry_after = crate::routing::retry_after(response.headers());
                let mut stream = response.bytes_stream();
                while let Some(chunk) = stream.next().await {
                    if let Ok(bytes) = chunk
                        && last_body.len() < 256 * 1024
                    {
                        let remaining = 256 * 1024 - last_body.len();
                        last_body.extend_from_slice(&bytes[..bytes.len().min(remaining)]);
                    }
                }
                timings.end_attempt(Outcome::HttpError);
                let kind = CheckKind::classify(status, &last_body);
                if kind == CheckKind::RateLimited {
                    crate::routing::start_cooldown(
                        &state,
                        candidate.channel_id,
                        &candidate.upstream_model,
                        retry_after,
                    );
                }
                route_failure = kind.traffic_failure(status);
                if kind.fails_over(status) {
                    continue;
                }
                stop = true;
                chain.abandon();
                continue;
            }
            if candidate.cooldown.strikes {
                crate::routing::clear_strikes(
                    &state,
                    candidate.channel_id,
                    &candidate.upstream_model,
                );
            }
            let ttfb_ms = started.elapsed().as_millis().min(i32::MAX as u128) as i32;
            let context = UsageContext {
                public_model: &public_model,
                protocol: &protocol,
                upstream_model: Some(&candidate.upstream_model),
                price,
                in_flight: is_stream,
                relayed_event_id: None,
                source: "gateway",
                stream: is_stream,
                input_chars,
                input_shapes: input_shapes.as_ref(),
            };
            let caller = (&key, context, (status.as_u16(), Some(channel_id)));
            if is_stream {
                let response_headers = response.headers().clone();
                let mut upstream = response.bytes_stream().boxed();
                // Held back until the first output, so a failover stays invisible to the client.
                let mut prefix = Vec::new();
                if let Some(deadline) = first_token_deadline {
                    let mut detector = crate::metering::OutputDetector::default();
                    let mut held = 0;
                    let missed = loop {
                        match tokio::time::timeout_at(deadline, upstream.next()).await {
                            Err(_) => break Some((Outcome::FirstTokenTimeout, first_token_missed)),
                            Ok(Some(Ok(bytes))) => {
                                let output = detector.push(&bytes);
                                held += bytes.len();
                                prefix.push(bytes);
                                if output {
                                    break None;
                                }
                                if held > MAX_UPSTREAM_BODY {
                                    break Some((
                                        Outcome::StreamError,
                                        "上游在输出前发送了过多数据".to_owned(),
                                    ));
                                }
                            }
                            Ok(Some(Err(_))) => {
                                break Some((
                                    Outcome::StreamError,
                                    "上游在输出前中断了响应".to_owned(),
                                ));
                            }
                            // An empty stream fails over, as a silent refusal; one
                            // that ended with an error event or usage is relayed as is.
                            Ok(None) if silent_end(&prefix) => {
                                break Some((
                                    Outcome::StreamError,
                                    "上游正常结束但没有任何输出和用量".to_owned(),
                                ));
                            }
                            Ok(None) => break None,
                        }
                    };
                    if let Some((outcome, reason)) = missed {
                        timings.end_attempt(outcome);
                        route_failure = Some((reason, None));
                        last_status = StatusCode::GATEWAY_TIMEOUT;
                        continue;
                    }
                }
                observe_traffic(&state, channel_id, health, None);
                let events =
                    record_route_usage(&state, caller, &relayed, (Some(ttfb_ms), ttfb_ms), None)
                        .await;
                timings.save(&state, &events).await;
                let mut permits: Vec<_> = permit.take().into_iter().collect();
                permits.extend(channel_permit);
                permits.extend(chain.take_permits());
                let mut output = Response::builder().status(status);
                copy_response_headers(&mut output, &response_headers);
                let accounting = StreamAccounting {
                    state: state.clone(),
                    events,
                    started,
                    ttfb_ms,
                    first_token_ms: None,
                    output_detector: crate::metering::OutputDetector::default(),
                    timings: Some(timings),
                    usage: None,
                    parser: StreamUsage::default(),
                    permits,
                };
                let (client, body) = tokio::sync::mpsc::channel(1);
                tokio::spawn(pump_stream(
                    stream::iter(prefix.into_iter().map(Ok))
                        .chain(upstream)
                        .boxed(),
                    accounting,
                    client,
                ));
                let stream = stream::unfold(body, |mut body| async move {
                    body.recv().await.map(|item| (item, body))
                });
                return output
                    .body(Body::from_stream(stream))
                    .map_err(|_| AppError::Internal("response construction failed".into()));
            }
            let upstream_body = match read_limited(response, MAX_UPSTREAM_BODY).await {
                Ok(body) => body,
                Err(error) => {
                    timings.finish(Outcome::StreamError);
                    let (key, context, (_, channel)) = caller;
                    let events = record_route_usage(
                        &state,
                        (
                            key,
                            context,
                            (StatusCode::INTERNAL_SERVER_ERROR.as_u16(), channel),
                        ),
                        &relayed,
                        (Some(ttfb_ms), elapsed_ms(started)),
                        None,
                    )
                    .await;
                    timings.save(&state, &events).await;
                    return Err(match error {
                        BodyError::TooLarge => AppError::Coded(
                            StatusCode::BAD_GATEWAY,
                            "upstream_response_too_large",
                            format!("上游响应超过 {} MiB 上限", MAX_UPSTREAM_BODY / 1024 / 1024),
                        ),
                        BodyError::Read => {
                            AppError::Internal("failed to read upstream response".into())
                        }
                    });
                }
            };
            timings.finish(Outcome::Success);
            observe_traffic(&state, channel_id, health, None);
            let usage = parse_usage(&upstream_body);
            let events = record_route_usage(
                &state,
                caller,
                &relayed,
                (
                    Some(ttfb_ms),
                    started.elapsed().as_millis().min(i32::MAX as u128) as i32,
                ),
                usage,
            )
            .await;
            timings.save(&state, &events).await;
            let mut output = Response::builder().status(status);
            output = output.header(header::CONTENT_TYPE, "application/json");
            return output
                .body(Body::from(upstream_body))
                .map_err(|_| AppError::Internal("response construction failed".into()));
        }
        if let Some(failure) = route_failure {
            observe_traffic(&state, channel_id, health, Some(failure));
        }
        if stop {
            break 'routes;
        }
    }
    if last_channel.is_none()
        && let Some(error) = skipped
    {
        return Err(error);
    }
    let event = record_usage(
        &state,
        &key,
        UsageContext {
            public_model: &public_model,
            protocol: &protocol,
            upstream_model: last_upstream_model.as_deref(),
            price: last_price,
            in_flight: false,
            relayed_event_id: last_relayed_event,
            source: "gateway",
            stream: is_stream,
            input_chars,
            input_shapes: input_shapes.as_ref(),
        },
        (last_status.as_u16(), last_channel),
        (
            None,
            started.elapsed().as_millis().min(i32::MAX as u128) as i32,
        ),
        None,
    )
    .await;
    timings.finish_routing();
    if let Some(event) = event {
        timings.save(&state, &[(event, key.account_id)]).await;
    }
    let content_type = if last_body.starts_with(b"{") {
        "application/json"
    } else {
        "text/plain; charset=utf-8"
    };
    Response::builder()
        .status(last_status)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(if last_body.is_empty() {
            b"upstream request failed".to_vec()
        } else {
            last_body
        }))
        .map_err(|_| AppError::Internal("response construction failed".into()))
}

/// Reads the virtual key from whichever header the client's native SDK uses:
/// `Authorization: Bearer` (OpenAI, Claude Code auth token), `x-api-key`
/// (Anthropic) or `x-goog-api-key` (Gemini).
pub(crate) fn client_token(headers: &HeaderMap) -> Option<&str> {
    let header_value = |name| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|v| !v.is_empty())
    };
    header_value(header::AUTHORIZATION.as_str())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .or_else(|| header_value("x-api-key"))
        .or_else(|| header_value("x-goog-api-key"))
}

fn invalid_key() -> AppError {
    AppError::Coded(
        StatusCode::UNAUTHORIZED,
        "invalid_api_key",
        format!("{PRODUCT} Key 无效或已撤销"),
    )
}

async fn authenticate_virtual_key(state: &AppState, headers: &HeaderMap) -> Result<GatewayKey> {
    let token = client_token(headers).ok_or_else(invalid_key)?;
    let hash = hash_token(token);
    let row = sqlx::query(&format!(
        "SELECT {KEY_COLUMNS} FROM virtual_api_keys v JOIN accounts o ON o.id=v.account_id \
         WHERE v.key_hash=$1 AND v.enabled=true AND v.revoked_at IS NULL AND o.disabled_at IS NULL"
    ))
    .bind(&hash)
    .fetch_optional(&state.db)
    .await?;
    if let Some(row) = row {
        return GatewayKey::from_row(&row);
    }
    // A paused key is still the holder's to use later, so say whom to ask;
    // unknown and revoked keys stay indistinguishable.
    let paused: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM virtual_api_keys WHERE key_hash=$1 AND revoked_at IS NULL)",
    )
    .bind(&hash)
    .fetch_one(&state.db)
    .await?;
    Err(if paused {
        AppError::Coded(
            StatusCode::UNAUTHORIZED,
            "key_paused",
            format!("此 {PRODUCT} Key 已暂停，请联系 Key 的发放方"),
        )
    } else {
        invalid_key()
    })
}

fn request_protocol_model(path: &str, payload: &Value) -> Result<(String, String)> {
    if path.starts_with("/v1beta/models/") {
        let model = path
            .trim_start_matches("/v1beta/models/")
            .split(':')
            .next()
            .unwrap_or("");
        if model.is_empty() {
            return Err(AppError::BadRequest("gemini model is missing".into()));
        }
        return Ok(("gemini".into(), model.into()));
    }
    if path == "/v1/chat/completions" || path == "/v1/responses" || path == "/v1/messages" {
        let model = payload
            .get("model")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| AppError::BadRequest("request body model is required".into()))?;
        return Ok((
            if path == "/v1/messages" {
                "anthropic"
            } else {
                "openai"
            }
            .into(),
            model.into(),
        ));
    }
    Err(AppError::NotFound)
}

async fn model_catalog(state: &AppState, key: &GatewayKey) -> Result<Response> {
    let mut catalog =
        models::routable_catalog_for(&state.db, key.account_id, key.channel_ids.as_deref()).await?;
    let mut budget = MAX_RELAY_EXPANSIONS;
    for (model, protocol) in relayed_models(state, key, &[key.account_id], &mut budget).await? {
        catalog.entry(model).or_insert(protocol);
    }
    let data: Vec<Value> = catalog
        .into_iter()
        .filter(|(model, _)| key.allows_model(model))
        .map(|(model, protocol)| json!({"id":model,"object":"model","owned_by":protocol}))
        .collect();
    Ok(Json(json!({"object":"list","data":data})).into_response())
}

/// Models reachable through the VAIR key channels in `key`'s account within
/// its channel scope, by name with the serving protocol. `path` lists the
/// accounts passed through, ending with the key's, as in `load_routes`; each
/// channel expanded spends one of `budget`.
fn relayed_models<'a>(
    state: &'a AppState,
    key: &'a GatewayKey,
    path: &'a [Uuid],
    budget: &'a mut usize,
) -> futures_util::future::BoxFuture<'a, Result<BTreeMap<String, String>>> {
    async move {
        let mut result = BTreeMap::new();
        // The account at the last hop routes only to its upstream channels.
        if path.len() > MAX_RELAY_HOPS {
            return Ok(result);
        }
        for relay in load_relay_channels(state, key.account_id)
            .await?
            .into_iter()
            .filter(|relay| {
                key.allows_channel(relay.channel_id) && !path.contains(&relay.source.account_id)
            })
        {
            if *budget == 0 {
                break;
            }
            *budget -= 1;
            let inner: Vec<Uuid> = path
                .iter()
                .copied()
                .chain([relay.source.account_id])
                .collect();
            let source_models = source_key_models(state, &relay.source, &inner, budget).await?;
            for (alias, target) in &relay.model_mapping {
                if let Some(protocol) = target.as_str().and_then(|target| source_models.get(target))
                {
                    result
                        .entry(alias.clone())
                        .or_insert_with(|| protocol.clone());
                }
            }
            for (model, protocol) in source_models {
                result.entry(model).or_insert(protocol);
            }
        }
        Ok(result)
    }
    .boxed()
}

/// Models a virtual key can reach through its account's upstream channels and,
/// while hops remain, its VAIR key channels, within its channel scope and model
/// permissions. `path` ends with the key's account (see `relayed_models`).
async fn source_key_models(
    state: &AppState,
    source: &GatewayKey,
    path: &[Uuid],
    budget: &mut usize,
) -> Result<BTreeMap<String, String>> {
    let mut models =
        models::routable_catalog_for(&state.db, source.account_id, source.channel_ids.as_deref())
            .await?;
    for (model, protocol) in relayed_models(state, source, path, budget).await? {
        models.entry(model).or_insert(protocol);
    }
    models.retain(|model, _| source.allows_model(model));
    Ok(models)
}

/// Whether `source` can serve `model` over `protocol` through its account's
/// upstream channels or, while hops remain, its VAIR key channels, without
/// consuming any limits. `path` and `budget` are as in `relayed_models`.
fn reachable<'a>(
    state: &'a AppState,
    source: &'a GatewayKey,
    protocol: &'a str,
    model: &'a str,
    path: &'a [Uuid],
    budget: &'a mut usize,
) -> futures_util::future::BoxFuture<'a, Result<bool>> {
    async move {
        if !model_rules::concrete(model) || !source.allows_model(model) {
            return Ok(false);
        }
        if !load_candidates(
            state,
            source.account_id,
            protocol,
            model,
            source.channel_ids.as_deref(),
        )
        .await?
        .is_empty()
        {
            return Ok(true);
        }
        if path.len() > MAX_RELAY_HOPS {
            return Ok(false);
        }
        for relay in load_relay_channels(state, source.account_id)
            .await?
            .into_iter()
            .filter(|relay| {
                source.allows_channel(relay.channel_id) && !path.contains(&relay.source.account_id)
            })
        {
            if *budget == 0 {
                break;
            }
            *budget -= 1;
            let model = relay
                .model_mapping
                .get(model)
                .and_then(Value::as_str)
                .unwrap_or(model);
            let inner: Vec<Uuid> = path
                .iter()
                .copied()
                .chain([relay.source.account_id])
                .collect();
            if reachable(state, &relay.source, protocol, model, &inner, budget).await? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    .boxed()
}

/// The model to check a relay channel with when none is configured: the one
/// its source key served most recently, else the first reachable one. A
/// wildcard rule such as `gpt-*` admits catalog names the upstream may not
/// serve, so the first name alone can fail a healthy channel.
async fn relay_check_model(
    state: &AppState,
    viewer: Uuid,
    source: &GatewayKey,
) -> Result<Option<String>> {
    let mut budget = MAX_RELAY_EXPANSIONS;
    let models: Vec<String> =
        source_key_models(state, source, &[viewer, source.account_id], &mut budget)
            .await?
            .into_keys()
            .collect();
    let recent: Option<String> = sqlx::query_scalar(
        "SELECT model FROM usage_events
         WHERE account_id=$1 AND virtual_key_id=$2 AND status BETWEEN 200 AND 299
           AND model=ANY($3) AND created_at>now()-interval '7 days'
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(source.account_id)
    .bind(source.id)
    .bind(&models)
    .fetch_optional(&state.db)
    .await?;
    Ok(recent.or_else(|| models.into_iter().next()))
}

/// Models currently reachable through an active source key, for display to
/// `viewer`, the account using it as a VAIR key channel.
pub(crate) async fn source_key_model_names(
    state: &AppState,
    key_id: Uuid,
    viewer: Uuid,
) -> Result<Vec<String>> {
    let row = sqlx::query(&format!(
        "SELECT {KEY_COLUMNS} FROM virtual_api_keys v JOIN accounts o ON o.id=v.account_id \
         WHERE v.id=$1 AND v.enabled=true AND v.revoked_at IS NULL AND o.disabled_at IS NULL"
    ))
    .bind(key_id)
    .fetch_optional(&state.db)
    .await?;
    let Some(row) = row else {
        return Ok(Vec::new());
    };
    let source = GatewayKey::from_row(&row)?;
    let mut budget = MAX_RELAY_EXPANSIONS;
    Ok(
        source_key_models(state, &source, &[viewer, source.account_id], &mut budget)
            .await?
            .into_keys()
            .collect(),
    )
}

/// Models a usable key can request, grouped by protocol like
/// `models::routable_by_protocol`: its account's upstream channels within the
/// key's scope, plus models reached through the account's VAIR key channels.
/// Empty for a paused or revoked key.
pub(crate) async fn key_models_by_protocol(
    state: &AppState,
    key_id: Uuid,
) -> Result<BTreeMap<String, BTreeSet<String>>> {
    let row = sqlx::query(&format!(
        "SELECT {KEY_COLUMNS} FROM virtual_api_keys v JOIN accounts o ON o.id=v.account_id \
         WHERE v.id=$1 AND v.enabled=true AND v.revoked_at IS NULL AND o.disabled_at IS NULL"
    ))
    .bind(key_id)
    .fetch_optional(&state.db)
    .await?;
    let Some(row) = row else {
        return Ok(BTreeMap::new());
    };
    let key = GatewayKey::from_row(&row)?;
    let mut result =
        models::routable_by_protocol(&state.db, key.account_id, key.channel_ids.as_deref()).await?;
    let mut budget = MAX_RELAY_EXPANSIONS;
    for (model, protocol) in relayed_models(state, &key, &[key.account_id], &mut budget).await? {
        result.entry(protocol).or_default().insert(model);
    }
    for names in result.values_mut() {
        names.retain(|model| key.allows_model(model));
    }
    result.retain(|_, names| !names.is_empty());
    Ok(result)
}

/// Relay channels in an account: enabled channels whose source key is usable.
async fn load_relay_channels(state: &AppState, account_id: Uuid) -> Result<Vec<RelayChannel>> {
    let rows = sqlx::query(&format!(
        "SELECT c.id AS channel_id, c.priority AS channel_priority, c.model_mapping, \
         {HEALTH_COLUMNS}, {KEY_COLUMNS} \
         FROM channels c JOIN virtual_api_keys v ON v.id=c.source_key_id \
         JOIN accounts o ON o.id=v.account_id \
         LEFT JOIN channel_health h ON h.channel_id=c.id \
         LEFT JOIN channel_monitor_configs mc ON mc.channel_id=c.id \
         WHERE c.account_id=$1 AND c.kind='vair_key' AND c.enabled=true \
           AND v.enabled=true AND v.revoked_at IS NULL AND v.account_id<>c.account_id \
           AND o.disabled_at IS NULL \
         ORDER BY c.priority ASC, c.created_at ASC"
    ))
    .bind(account_id)
    .fetch_all(&state.db)
    .await?;
    rows.iter()
        .map(|row| {
            let invalid = |_| AppError::Internal("invalid channel".into());
            let mapping: Value = row.try_get("model_mapping").map_err(invalid)?;
            Ok(RelayChannel {
                channel_id: row.try_get("channel_id").map_err(invalid)?,
                priority: row.try_get("channel_priority").map_err(invalid)?,
                model_mapping: mapping.as_object().cloned().unwrap_or_default(),
                source: GatewayKey::from_row(row)?,
                health: ChannelHealth::from_row(row)?,
            })
        })
        .collect()
}

/// Upstream candidates and relay channels for a request, in priority order,
/// within the key's channel scope. `path` lists the accounts the request has
/// passed through, ending with the key's; relay channels back into them are
/// left out, and `None` leaves out relay channels altogether.
async fn load_routes(
    state: &AppState,
    key: &GatewayKey,
    protocol: &str,
    model: &str,
    path: Option<&[Uuid]>,
) -> Result<Vec<Route>> {
    let mut ranked: Vec<(Rank, Route)> = load_ranked_candidates(
        state,
        key.account_id,
        protocol,
        model,
        key.channel_ids.as_deref(),
    )
    .await?
    .into_iter()
    .map(|(rank, candidate)| (rank, Route::Upstream(candidate)))
    .collect();
    let relays: Vec<RelayChannel> = match path {
        Some(path) => load_relay_channels(state, key.account_id)
            .await?
            .into_iter()
            .filter(|relay| {
                key.allows_channel(relay.channel_id) && !path.contains(&relay.source.account_id)
            })
            .collect(),
        None => Vec::new(),
    };
    // Quality monitors on relay channels steer routing as they do for upstream channels.
    let degraded: Vec<(Uuid, String)> = if relays.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as(
            "SELECT channel_id,route_policy FROM quality_monitors
             WHERE channel_id=ANY($1) AND model=$2 AND kind='service' AND deleted_at IS NULL
               AND status='degraded' AND valid_until>now() AND route_policy IN ('exclude','deprioritize')
             UNION ALL
             SELECT channel_id,CASE WHEN route_policy='exclude' AND window_verdict='confirmed' THEN 'exclude' ELSE 'deprioritize' END
             FROM quality_monitors
             WHERE channel_id=ANY($1) AND model=$2 AND kind='authenticity' AND deleted_at IS NULL
               AND window_verdict IN ('confirmed','suspected') AND window_valid_until>now()
               AND route_policy IN ('exclude','deprioritize')",
        )
        .bind(relays.iter().map(|relay| relay.channel_id).collect::<Vec<_>>())
        .bind(model)
        .fetch_all(&state.db)
        .await?
    };
    for relay in relays {
        // Service checks and authenticity may both apply; exclusion wins.
        let policies: Vec<&str> = degraded
            .iter()
            .filter(|(id, _)| *id == relay.channel_id)
            .map(|(_, policy)| policy.as_str())
            .collect();
        if policies.contains(&"exclude") {
            continue;
        }
        let deprioritized = !policies.is_empty();
        let model = relay
            .model_mapping
            .get(model)
            .and_then(Value::as_str)
            .unwrap_or(model)
            .to_owned();
        // Cooldowns apply to the upstream channels behind the relay, inside it.
        ranked.push((
            (relay.health.excluded, false, deprioritized, relay.priority),
            Route::Relay {
                channel_id: relay.channel_id,
                model,
                source: relay.source,
                health: relay.health,
            },
        ));
    }
    // Stable: upstream candidates keep their tie-breaking order from SQL.
    ranked.sort_by_key(|(rank, _)| *rank);
    Ok(ranked.into_iter().map(|(_, route)| route).collect())
}

/// Applies the source key's model permissions, channel scope and every limit to
/// a relayed request reaching it through `channel_id`, and loads the routes of
/// its account along `path` (see `load_routes`). An inner `Err` makes the relay
/// channel unavailable so the caller fails over.
async fn prepare_relay(
    state: &AppState,
    protocol: &str,
    channel_id: Uuid,
    source: GatewayKey,
    model: String,
    path: Option<&[Uuid]>,
) -> Result<std::result::Result<(Vec<Route>, Relay), (StatusCode, &'static str)>> {
    let unavailable = RELAY_UNAVAILABLE;
    let limited = (
        StatusCode::TOO_MANY_REQUESTS,
        "VAIR key channel limit reached",
    );
    if !model_rules::concrete(&model) || !source.allows_model(&model) {
        return Ok(Err(unavailable));
    }
    let routes = load_routes(state, &source, protocol, &model, path).await?;
    if routes.is_empty() {
        return Ok(Err(unavailable));
    }
    // A limit or an unknown-cost pause on the source key makes the channel unavailable.
    let unusable = |result: Result<()>| match result {
        Ok(()) => Ok(false),
        Err(AppError::RateLimited) => Ok(true),
        Err(AppError::Coded(status, ..))
            if status == StatusCode::TOO_MANY_REQUESTS || status == StatusCode::CONFLICT =>
        {
            Ok(true)
        }
        Err(error) => Err(error),
    };
    if unusable(
        state
            .redis_rate_limit(source.id, source.requests_per_minute)
            .await,
    )? || unusable(enforce_limits(state, &source).await)?
    {
        return Ok(Err(limited));
    }
    let semaphore = state
        .semaphore(source.id, source.concurrency_limit.max(1) as usize)
        .await;
    // The permit is taken without waiting, so the limits just checked still hold.
    let Ok(permit) = semaphore.try_acquire_owned() else {
        return Ok(Err(limited));
    };
    Ok(Ok((
        routes,
        Relay {
            channel_id,
            source,
            model,
            permit,
        },
    )))
}

async fn load_candidates(
    state: &AppState,
    account_id: Uuid,
    protocol: &str,
    model: &str,
    channels: Option<&[Uuid]>,
) -> Result<Vec<Candidate>> {
    Ok(
        load_ranked_candidates(state, account_id, protocol, model, channels)
            .await?
            .into_iter()
            .map(|(_, candidate)| candidate)
            .collect(),
    )
}

/// Route sort key: (unavailable, cooling down after a rate limit, deprioritized
/// by quality, priority).
type Rank = (bool, bool, bool, i32);

/// Upstream candidates with their sort key, in routing order.
async fn load_ranked_candidates(
    state: &AppState,
    account_id: Uuid,
    protocol: &str,
    model: &str,
    channels: Option<&[Uuid]>,
) -> Result<Vec<(Rank, Candidate)>> {
    let rows = sqlx::query(&format!(
        "SELECT {HEALTH_COLUMNS}, c.id AS channel_id, c.priority AS channel_priority, c.base_url, c.protocol, c.concurrency_limit, \
         COALESCE(c.model_mapping ->> $2, r.upstream_model, $2) AS upstream_model, ck.secret_ciphertext, \
         c.models,c.model_mode,(r.upstream_model IS NOT NULL OR c.model_mapping ? $2) AS explicit_mapping, \
         (COALESCE(qm.route_policy='deprioritize' AND qm.status='degraded' AND qm.valid_until>now(),false) \
          OR COALESCE(am.window_valid_until>now() AND am.window_verdict IN ('confirmed','suspected') \
             AND (am.route_policy='deprioritize' OR (am.route_policy='exclude' AND am.window_verdict='suspected')),false)) AS deprioritized, \
         COALESCE(r.priority, c.priority) AS effective_priority \
         FROM channels c JOIN channel_keys ck ON ck.channel_id=c.id \
         LEFT JOIN LATERAL (SELECT r.upstream_model,r.priority FROM model_routes r \
           WHERE r.channel_id=c.id AND r.account_id=$1 AND r.public_model=$2 AND r.protocol=$3 AND r.enabled=true \
           ORDER BY r.priority ASC, r.id LIMIT 1) r ON true \
         LEFT JOIN channel_health h ON h.channel_id=c.id \
         LEFT JOIN channel_monitor_configs mc ON mc.channel_id=c.id \
         LEFT JOIN quality_monitors qm ON qm.channel_id=c.id AND qm.model=$2 AND qm.kind='service' AND qm.deleted_at IS NULL \
         LEFT JOIN quality_monitors am ON am.channel_id=c.id AND am.model=$2 AND am.kind='authenticity' AND am.deleted_at IS NULL \
         WHERE c.account_id=$1 AND c.kind='upstream' AND c.protocol=$3 AND c.enabled=true AND ck.enabled=true \
           AND ($4::uuid[] IS NULL OR c.id=ANY($4)) \
           AND NOT COALESCE(qm.route_policy='exclude' AND qm.status='degraded' AND qm.valid_until>now(),false) \
           AND NOT COALESCE(am.route_policy='exclude' AND am.window_verdict='confirmed' AND am.window_valid_until>now(),false) \
         ORDER BY availability_excluded ASC, deprioritized ASC, effective_priority ASC, c.priority ASC, ck.created_at ASC"
    ))
    .bind(account_id)
    .bind(model)
    .bind(protocol)
    .bind(channels)
    .fetch_all(&state.db)
    .await?;
    let mut result = Vec::new();
    for row in rows {
        let rules: Vec<String> = serde_json::from_value(row.try_get("models")?)
            .map_err(|_| AppError::Internal("invalid channel rules".into()))?;
        if !row.try_get::<bool, _>("explicit_mapping")?
            && row.try_get::<String, _>("model_mode")? != "passthrough"
            && !model_rules::allows(&rules, model)
        {
            continue;
        }
        let ciphertext: String = row
            .try_get("secret_ciphertext")
            .map_err(|_| AppError::Internal("invalid channel key".into()))?;
        let secret = state
            .crypto
            .decrypt(&ciphertext)
            .map_err(|_| AppError::Internal("channel credential could not be decrypted".into()))?;
        let health = ChannelHealth::from_row(&row)?;
        let rank = (
            health.excluded,
            false,
            row.try_get("deprioritized")
                .map_err(|_| AppError::Internal("invalid channel".into()))?,
            row.try_get("effective_priority")
                .map_err(|_| AppError::Internal("invalid channel".into()))?,
        );
        result.push((
            rank,
            Candidate {
                channel_id: row
                    .try_get("channel_id")
                    .map_err(|_| AppError::Internal("invalid channel".into()))?,
                base_url: row
                    .try_get("base_url")
                    .map_err(|_| AppError::Internal("invalid channel".into()))?,
                protocol: row
                    .try_get("protocol")
                    .map_err(|_| AppError::Internal("invalid channel".into()))?,
                upstream_model: row
                    .try_get("upstream_model")
                    .map_err(|_| AppError::Internal("invalid route".into()))?,
                secret,
                health,
                cooldown: crate::routing::Cooldown::default(),
                concurrency_limit: row
                    .try_get("concurrency_limit")
                    .map_err(|_| AppError::Internal("invalid channel".into()))?,
            },
        ));
    }
    let targets: Vec<(Uuid, &str)> = result
        .iter()
        .map(|(_, candidate)| (candidate.channel_id, candidate.upstream_model.as_str()))
        .collect();
    let cooldowns = crate::routing::cooldowns(state, &targets).await;
    for ((rank, candidate), cooldown) in result.iter_mut().zip(cooldowns) {
        rank.1 = cooldown.cooling;
        candidate.cooldown = cooldown;
    }
    // Stable: candidates keep their tie-breaking order from SQL.
    result.sort_by_key(|(rank, _)| *rank);
    Ok(result)
}

fn rewrite_model(payload: &mut Value, protocol: &str, upstream_model: &str) {
    if (protocol == "openai" || protocol == "anthropic")
        && let Some(object) = payload.as_object_mut()
    {
        object.insert("model".into(), Value::String(upstream_model.into()));
    }
}

fn upstream_path(path: &str, protocol: &str, public_model: &str, upstream_model: &str) -> String {
    if protocol == "gemini" {
        return path.replacen(
            &format!("models/{public_model}"),
            &format!("models/{upstream_model}"),
            1,
        );
    }
    path.to_owned()
}

pub(crate) fn join_base_url(base: &str, path: &str) -> Result<String> {
    let base = base.trim_end_matches('/');
    // OpenAI-compatible upstreams may version their root other than `/v1`,
    // e.g. Zhipu `/api/paas/v4` or Volcengine Ark `/api/v3`; the base already
    // carries the version, so the request's `/v1` is dropped.
    let versioned = base
        .rsplit('/')
        .next()
        .and_then(|segment| segment.strip_prefix('v'))
        .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()));
    let path = if versioned && path.starts_with("/v1/") {
        &path[3..]
    } else if base.ends_with("/v1beta") && path.starts_with("/v1beta/") {
        &path[7..]
    } else {
        path
    };
    Ok(format!("{base}{path}"))
}

/// Appends the client's query string (e.g. Gemini `alt=sse`) to the upstream
/// target. `key` is dropped because it would carry the VAIR key upstream.
fn append_client_query(target: &str, query: Option<&str>) -> String {
    let forwarded = query
        .unwrap_or("")
        .split('&')
        .filter(|pair| !pair.is_empty() && pair.split('=').next() != Some("key"))
        .collect::<Vec<_>>()
        .join("&");
    if forwarded.is_empty() {
        target.to_owned()
    } else if target.contains('?') {
        format!("{target}&{forwarded}")
    } else {
        format!("{target}?{forwarded}")
    }
}

/// Whether channels may point at private networks. Set once at startup.
static ALLOW_PRIVATE_UPSTREAMS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Lets channels reach private and reserved networks, such as a model served
/// on the same host or a gateway inside a company network. Only a
/// self-hosted instance allows it: there the instance owner is the only one
/// who configures channels, so a private target exposes nothing they could
/// not reach already.
pub fn allow_private_upstreams(allow: bool) {
    ALLOW_PRIVATE_UPSTREAMS.store(
        allow && !crate::edition::CLOUD,
        std::sync::atomic::Ordering::Relaxed,
    );
}

pub(crate) fn private_upstreams_allowed() -> bool {
    ALLOW_PRIVATE_UPSTREAMS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Whether a channel host is refused before it is resolved: local names, and
/// addresses on a private or reserved network unless those are allowed.
pub(crate) fn blocked_host(host: &str, allow_private: bool) -> bool {
    if allow_private {
        return false;
    }
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
    {
        return true;
    }
    host.parse::<IpAddr>().is_ok_and(is_private_or_reserved)
}

pub(crate) async fn ensure_safe_target(target: &str) -> Result<()> {
    let url = url::Url::parse(target)
        .map_err(|_| AppError::BadRequest("channel URL is invalid".into()))?;
    let host = url
        .host_str()
        .ok_or_else(|| AppError::BadRequest("channel URL has no host".into()))?;
    let port = url.port_or_known_default().ok_or_else(|| {
        AppError::BadRequest("channel URL must use a known HTTP or HTTPS port".into())
    })?;
    let allow_private = private_upstreams_allowed();
    let private =
        || AppError::BadRequest("channel URL resolves to a private or reserved network".into());
    if blocked_host(host, allow_private) {
        return Err(private());
    }
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| AppError::BadRequest("channel host could not be resolved".into()))?;
    let mut found = false;
    for address in addresses {
        found = true;
        if !allow_private && is_private_or_reserved(address.ip()) {
            return Err(private());
        }
    }
    if !found {
        return Err(AppError::BadRequest("channel host has no addresses".into()));
    }
    Ok(())
}

/// Resolves upstream hosts for the shared HTTP client, dropping private and
/// reserved addresses at connect time. The check before a request resolves
/// the name separately, so without this a host could answer it with a public
/// address and the connection with a private one.
pub(crate) struct SafeResolver {
    // Proxy hosts from the environment, which the operator chose.
    trusted: HashSet<String>,
}

impl SafeResolver {
    pub(crate) fn from_env() -> Self {
        let trusted = [
            "HTTP_PROXY",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
        ]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .filter_map(|value| url::Url::parse(&value).ok())
        .filter_map(|url| url.host_str().map(str::to_ascii_lowercase))
        .collect();
        Self { trusted }
    }
}

impl reqwest::dns::Resolve for SafeResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_ascii_lowercase();
        let filter = !private_upstreams_allowed() && !self.trusted.contains(&host);
        Box::pin(async move {
            let addresses: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host, 0))
                .await?
                .filter(|address| !filter || !is_private_or_reserved(address.ip()))
                .collect();
            if addresses.is_empty() {
                return Err("host resolves only to private or reserved addresses".into());
            }
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// Addresses no channel may reach unless private upstreams are allowed:
/// private, loopback, link-local, shared, documentation, benchmarking,
/// multicast and reserved ranges, including IPv4 written inside IPv6.
pub(crate) fn is_private_or_reserved(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_broadcast()
                || a == 0
                || a >= 240
                || a == 100 && (b & 0b1100_0000) == 0b0100_0000
                || a == 192 && b == 0 && (c == 0 || c == 2)
                || a == 192 && b == 88 && c == 99
                || a == 198 && (b == 18 || b == 19)
                || a == 198 && b == 51 && c == 100
                || a == 203 && b == 0 && c == 113
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            let embedded = |high: u16, low: u16| {
                IpAddr::V4(std::net::Ipv4Addr::from(
                    (u32::from(high) << 16) | u32::from(low),
                ))
            };
            if let Some(v4) = ip.to_ipv4_mapped() {
                return is_private_or_reserved(IpAddr::V4(v4));
            }
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                // IPv4-compatible ::/96, deprecated and never routed.
                || segments[..6] == [0; 6]
                // Unique local fc00::/7, link-local fe80::/10, site-local fec0::/10.
                || segments[0] & 0xfe00 == 0xfc00
                || segments[0] & 0xffc0 == 0xfe80
                || segments[0] & 0xffc0 == 0xfec0
                // Discard 100::/64 and documentation 2001:db8::/32.
                || segments[..4] == [0x100, 0, 0, 0]
                || segments[0] == 0x2001 && segments[1] == 0xdb8
                // Teredo 2001::/32 tunnels to an address hidden in the packet.
                || segments[0] == 0x2001 && segments[1] == 0
                // NAT64 64:ff9b::/96 and 6to4 2002::/16 carry an IPv4 address.
                || segments[..6] == [0x64, 0xff9b, 0, 0, 0, 0]
                    && is_private_or_reserved(embedded(segments[6], segments[7]))
                || segments[0] == 0x2002 && is_private_or_reserved(embedded(segments[1], segments[2]))
        }
    }
}

fn model_not_supported(status: StatusCode, body: &[u8]) -> bool {
    if status != StatusCode::BAD_REQUEST && status != StatusCode::NOT_FOUND {
        return false;
    }
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return false;
    };
    let error = &value["error"];
    // Anthropic: {"type":"error","error":{"type":"not_found_error","message":"model: <id>"}}
    if status == StatusCode::NOT_FOUND
        && error["type"] == "not_found_error"
        && error["message"]
            .as_str()
            .is_some_and(|message| message.starts_with("model:"))
    {
        return true;
    }
    let code = error["code"].as_str().unwrap_or("");
    if [
        "model_not_found",
        "model_not_supported",
        "unsupported_model",
        "invalid_model",
    ]
    .contains(&code)
    {
        return true;
    }
    let message = error["message"].as_str().unwrap_or("").to_ascii_lowercase();
    message.contains("model")
        && (message.contains("model not found")
            || message.contains("model does not exist")
            || message.contains("model is not supported")
            || (error["status"] == "NOT_FOUND" && message.contains("not found")))
}

fn copy_response_headers(builder: &mut axum::http::response::Builder, headers: &HeaderMap) {
    for (name, value) in headers {
        if *name == header::CONTENT_LENGTH
            || *name == header::TRANSFER_ENCODING
            || *name == header::CONNECTION
        {
            continue;
        }
        if let Ok(value) = HeaderValue::from_bytes(value.as_bytes()) {
            *builder = std::mem::take(builder).header(name, value);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TokenUsage {
    input: i64,
    output: i64,
    // None means the upstream did not report cache reads; Some(0) is a reported miss.
    cached: Option<i64>,
}

impl TokenUsage {
    pub(crate) fn input(&self) -> i64 {
        self.input
    }

    pub(crate) fn output(&self) -> i64 {
        self.output
    }
}

pub(crate) fn parse_usage(body: &[u8]) -> Option<TokenUsage> {
    let value: Value = serde_json::from_slice(body).ok()?;
    // OpenAI-compatible upstreams may send `"usage": null`; that is missing
    // usage, not zero tokens.
    let usage = [
        "/usage",
        "/usageMetadata",
        "/message/usage",
        "/response/usage",
        "/response/usageMetadata",
    ]
    .into_iter()
    .filter_map(|path| value.pointer(path))
    .find(|usage| usage.is_object())?;
    let input = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .or_else(|| usage.get("promptTokenCount"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let output = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .or_else(|| usage.get("candidatesTokenCount"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let cache_read = usage.get("cache_read_input_tokens").and_then(Value::as_i64);
    let cache_creation = usage
        .get("cache_creation_input_tokens")
        .and_then(Value::as_i64);
    let cached = usage
        .pointer("/prompt_tokens_details/cached_tokens")
        .or_else(|| usage.pointer("/input_tokens_details/cached_tokens"))
        .or_else(|| usage.get("cache_read_input_tokens"))
        .or_else(|| usage.get("cachedContentTokenCount"))
        // Older DeepSeek and Kimi responses report cache hits only at the top
        // level of `usage`.
        .or_else(|| usage.get("prompt_cache_hit_tokens"))
        .or_else(|| usage.get("cached_tokens"))
        .and_then(Value::as_i64)
        // Gemini omits `cachedContentTokenCount` on a cache miss instead of
        // reporting zero, so its usage metadata always counts as reported.
        .or_else(|| usage.get("promptTokenCount").map(|_| 0));
    // Anthropic reports `input_tokens` as uncached input and exposes cache reads
    // and writes as separate fields. Normalize those fields so the stored input
    // count represents all input tokens, matching OpenAI and Gemini semantics.
    let input = input
        .saturating_add(cache_read.unwrap_or(0))
        .saturating_add(cache_creation.unwrap_or(0));
    Some(TokenUsage {
        input,
        output,
        cached,
    })
}

/// Credentials and connection headers are replaced per upstream. The client's
/// Accept-Encoding is dropped as well: usage accounting must read response
/// bodies, so reqwest negotiates compression itself and decodes the response.
fn forwards_client_header(name: &HeaderName) -> bool {
    !(name == header::AUTHORIZATION
        || name == header::HOST
        || name == header::CONTENT_LENGTH
        || name == header::COOKIE
        || name == header::ACCEPT_ENCODING
        || name == "x-api-key"
        || name == "x-goog-api-key")
}

pub(crate) fn parse_stream_usage(body: &[u8]) -> Option<TokenUsage> {
    let mut usage = StreamUsage::default();
    usage.push(body);
    usage.finish()
}

/// Longest single stream line parsed for usage; longer ones are content.
const MAX_STREAM_LINE: usize = 1024 * 1024;

/// Reads the usage a stream reports chunk by chunk, keeping only the
/// unfinished line instead of the whole stream.
#[derive(Default)]
pub(crate) struct StreamUsage {
    line: Vec<u8>,
    // The current line outgrew MAX_STREAM_LINE and is skipped to its end.
    overlong: bool,
    // The stream so far while no line carried usage, for Gemini JSON arrays;
    // None once it outgrew MAX_METADATA_BODY.
    whole: Option<Vec<u8>>,
    started: bool,
    usage: Option<TokenUsage>,
}

impl StreamUsage {
    /// The usage reported by the complete lines so far.
    pub(crate) fn usage(&self) -> Option<TokenUsage> {
        self.usage
    }

    pub(crate) fn push(&mut self, chunk: &[u8]) {
        if !self.started {
            self.started = true;
            self.whole = Some(Vec::new());
        }
        if let Some(whole) = &mut self.whole {
            if self.usage.is_some() || whole.len() + chunk.len() > MAX_METADATA_BODY {
                self.whole = None;
            } else {
                whole.extend_from_slice(chunk);
            }
        }
        let mut rest = chunk;
        while let Some(end) = rest.iter().position(|byte| *byte == b'\n') {
            if !self.overlong && self.line.len() + end <= MAX_STREAM_LINE {
                self.line.extend_from_slice(&rest[..end]);
                let line = std::mem::take(&mut self.line);
                self.parse_line(&line);
            }
            self.line.clear();
            self.overlong = false;
            rest = &rest[end + 1..];
        }
        if !self.overlong {
            if self.line.len() + rest.len() > MAX_STREAM_LINE {
                self.overlong = true;
                self.line = Vec::new();
            } else {
                self.line.extend_from_slice(rest);
            }
        }
    }

    /// The stream's usage once it ended, counting an unterminated last line.
    pub(crate) fn finish(mut self) -> Option<TokenUsage> {
        if !self.overlong {
            let line = std::mem::take(&mut self.line);
            self.parse_line(&line);
        }
        // Gemini without `alt=sse` streams one JSON array of pretty-printed
        // responses, which only parses once the array is complete.
        if self.usage.is_none()
            && let Some(whole) = &self.whole
            && let Ok(items) = serde_json::from_slice::<Vec<Value>>(whole.trim_ascii())
        {
            for item in items {
                if let Some(usage) = serde_json::to_vec(&item)
                    .ok()
                    .and_then(|bytes| parse_usage(&bytes))
                {
                    self.merge(usage);
                }
            }
        }
        self.usage
    }

    fn parse_line(&mut self, line: &[u8]) {
        let line = line.strip_prefix(b"data:").unwrap_or(line).trim_ascii();
        if line.is_empty() || line == b"[DONE]" {
            return;
        }
        if let Some(usage) = parse_usage(line) {
            self.merge(usage);
        }
    }

    fn merge(&mut self, usage: TokenUsage) {
        self.usage = Some(match self.usage {
            Some(previous) => TokenUsage {
                input: previous.input.max(usage.input),
                output: previous.output.max(usage.output),
                cached: previous.cached.max(usage.cached),
            },
            None => usage,
        });
    }
}

#[derive(Clone, Copy)]
struct UsageContext<'a> {
    public_model: &'a str,
    protocol: &'a str,
    upstream_model: Option<&'a str>,
    price: Option<&'a pricing::ModelPrice>,
    in_flight: bool,
    // The source key's event when this row is the caller's side of a relay.
    relayed_event_id: Option<Uuid>,
    // 'gateway' for client traffic, 'monitor' for probes through a VAIR key channel.
    source: &'static str,
    stream: bool,
    // (ASCII, other) characters of request text, for passive metering.
    input_chars: Option<(i32, i32)>,
    // Local token counts of the request, for passive metering.
    input_shapes: Option<&'a crate::token_estimate::Shapes>,
}

async fn record_usage(
    state: &AppState,
    key: &GatewayKey,
    context: UsageContext<'_>,
    outcome: (u16, Option<Uuid>),
    timing: (Option<i32>, i32),
    usage: Option<TokenUsage>,
) -> Option<Uuid> {
    let (input, output, cached) = usage.map_or((None, None, None), |v| {
        (Some(v.input), Some(v.output), v.cached)
    });
    let cost = context
        .price
        .zip(usage)
        .map(|(p, u)| p.estimate(u.input, u.output, u.cached.unwrap_or(0)));
    let pricing_status = if cost.is_some() {
        "priced"
    } else if context.price.is_some() {
        "pending_usage"
    } else {
        "unpriced"
    };
    // Only answered requests are metered. Counting usually finishes while the
    // request is upstream; otherwise the shape is filled in once it does.
    let shapes = context
        .input_shapes
        .zip(context.upstream_model)
        .filter(|_| (200..300).contains(&outcome.0));
    let ready = shapes.and_then(|(shapes, _)| shapes.clone().now_or_never());
    let pending = shapes.is_some() && ready.is_none();
    let input_shape = ready
        .flatten()
        .zip(context.upstream_model)
        .and_then(|(shapes, model)| crate::token_estimate::shape_for(&shapes, model));
    let id: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO usage_events(account_id,virtual_key_id,model,protocol,status,ttfb_ms,duration_ms,
         input_tokens,output_tokens,cached_tokens,channel_id,cache_usage_reported,upstream_model,price_snapshot,cost_usd,pricing_status,in_flight_until,
         relayed_event_id,source,stream,input_ascii_chars,input_other_chars,input_shape)
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,
           CASE WHEN $17 THEN now()+make_interval(secs=>$18) END,$19,$20,$21,$22,$23,$24) RETURNING id"
    ).bind(key.account_id).bind(key.id).bind(context.public_model).bind(context.protocol)
        .bind(outcome.0 as i32).bind(timing.0).bind(timing.1).bind(input).bind(output).bind(cached)
        .bind(outcome.1).bind(cached.is_some()).bind(context.upstream_model)
        .bind(context.price.map(|p|json!(p))).bind(cost).bind(pricing_status)
        .bind(context.in_flight).bind(UPSTREAM_TIMEOUT.as_secs_f64())
        .bind(context.relayed_event_id).bind(context.source).bind(context.stream)
        .bind(context.input_chars.map(|c| c.0)).bind(context.input_chars.map(|c| c.1))
        .bind(input_shape)
        .fetch_optional(&state.db).await.ok().flatten();
    if let (Some(id), Some((shapes, model)), true) = (id, shapes, pending) {
        let (db, shapes, model) = (state.db.clone(), shapes.clone(), model.to_owned());
        tokio::spawn(async move {
            let shape = shapes
                .await
                .and_then(|shapes| crate::token_estimate::shape_for(&shapes, &model));
            if let Some(shape) = shape {
                let _ = sqlx::query("UPDATE usage_events SET input_shape=$1 WHERE id=$2")
                    .bind(shape)
                    .bind(id)
                    .execute(&db)
                    .await;
            }
        });
    }
    id
}

/// Records the caller's usage. A relayed request first records each source
/// key's usage, innermost first, against the channel in its account that served
/// it, so it counts toward that key's limits, and each row points at the one below. Returns (event, account)
/// pairs for stream accounting.
async fn record_route_usage(
    state: &AppState,
    caller: (&GatewayKey, UsageContext<'_>, (u16, Option<Uuid>)),
    relayed: &[(&Relay, Uuid)],
    timing: (Option<i32>, i32),
    usage: Option<TokenUsage>,
) -> Vec<(Uuid, Uuid)> {
    let (key, context, (status, channel_id)) = caller;
    let mut events = Vec::new();
    let mut relayed_event_id = None;
    // Innermost first, so each hop's row points at the one it relayed to.
    for &(relay, channel) in relayed.iter().rev() {
        relayed_event_id = record_usage(
            state,
            &relay.source,
            UsageContext {
                public_model: &relay.model,
                relayed_event_id,
                ..context
            },
            (status, Some(channel)),
            timing,
            usage,
        )
        .await;
        if let Some(id) = relayed_event_id {
            events.push((id, relay.source.account_id));
        }
    }
    if let Some(id) = record_usage(
        state,
        key,
        UsageContext {
            relayed_event_id,
            ..context
        },
        (status, channel_id),
        timing,
        usage,
    )
    .await
    {
        events.push((id, key.account_id));
    }
    events
}

/// Records a failed request on a relay hop's source key, against the channel of
/// its last attempt, linked to the failure one hop further in.
async fn record_relay_failure(
    state: &AppState,
    frame: &RelayFrame,
    context: UsageContext<'_>,
    status: u16,
    duration_ms: i32,
) -> Option<Uuid> {
    let (channel, relayed_event_id) = frame.last?;
    record_usage(
        state,
        &frame.relay.source,
        UsageContext {
            public_model: &frame.relay.model,
            relayed_event_id,
            ..context
        },
        (status, Some(channel)),
        (None, duration_ms),
        None,
    )
    .await
}

/// How long a stream whose client has gone may idle before the upstream is
/// dropped; the upstream timeout still bounds the whole request.
const DRAIN_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Relays an upstream stream to the client and records its usage. After the
/// client disconnects, the upstream is read to the end, so a cancelled
/// request is billed by the usage the upstream reports rather than left
/// without a cost.
async fn pump_stream(
    mut upstream: stream::BoxStream<'static, reqwest::Result<axum::body::Bytes>>,
    mut accounting: StreamAccounting,
    client: tokio::sync::mpsc::Sender<std::io::Result<axum::body::Bytes>>,
) {
    let mut client = Some(client);
    let mut ended = false;
    let outcome = loop {
        let next = if client.is_some() {
            upstream.next().await
        } else {
            match tokio::time::timeout(DRAIN_IDLE_TIMEOUT, upstream.next()).await {
                Ok(next) => next,
                Err(_) => break Outcome::ClientDisconnected,
            }
        };
        match next {
            Some(Ok(bytes)) => {
                accounting.observe(&bytes);
                if let Some(sender) = &client
                    && sender.send(Ok(bytes)).await.is_err()
                {
                    client = None;
                }
            }
            Some(Err(error)) => match client.take() {
                Some(sender) => {
                    let _ = sender.send(Err(std::io::Error::other(error))).await;
                    break Outcome::StreamError;
                }
                None => break Outcome::ClientDisconnected,
            },
            None => {
                ended = true;
                break if client.is_some() {
                    Outcome::Success
                } else {
                    Outcome::ClientDisconnected
                };
            }
        }
    };
    accounting.usage = std::mem::take(&mut accounting.parser)
        .finish()
        .or(accounting.usage);
    // A stream that ended without any output or usage generated nothing to bill.
    if ended && accounting.first_token_ms.is_none() && accounting.usage.is_none() {
        accounting.usage = Some(TokenUsage {
            input: 0,
            output: 0,
            cached: None,
        });
    }
    accounting.finish(outcome).await;
}

/// Whether a stream that ended before any output also carried no usage and
/// no error, i.e. the upstream silently refused instead of answering.
fn silent_end(prefix: &[axum::body::Bytes]) -> bool {
    let body = prefix.concat();
    parse_stream_usage(&body).is_none()
        && !body.windows(5).any(|w| w == b"error")
        && !body.windows(15).any(|w| w == b"response.failed")
        && !body.windows(19).any(|w| w == b"response.incomplete")
}

struct StreamAccounting {
    state: AppState,
    // (event, account) pairs: the caller's event and, for a relay, each source key's.
    events: Vec<(Uuid, Uuid)>,
    started: Instant,
    ttfb_ms: i32,
    // When generated output first arrived, for output speed.
    first_token_ms: Option<i32>,
    output_detector: crate::metering::OutputDetector,
    timings: Option<Timings>,
    usage: Option<TokenUsage>,
    parser: StreamUsage,
    permits: Vec<OwnedSemaphorePermit>,
}

impl StreamAccounting {
    /// Records the first output and any new usage once `chunk` arrives.
    fn observe(&mut self, chunk: &[u8]) {
        if self.first_token_ms.is_none() && self.output_detector.push(chunk) {
            let first_token_ms = elapsed_ms(self.started);
            self.first_token_ms = Some(first_token_ms);
            if let Some(timings) = &mut self.timings {
                timings.first_output(first_token_ms);
            }
        }
        self.parser.push(chunk);
        let usage = self.parser.usage().or(self.usage);
        if usage != self.usage {
            self.usage = usage;
            for &(event_id, account_id) in &self.events {
                tokio::spawn(update_usage_event(
                    self.state.clone(),
                    event_id,
                    account_id,
                    Some(self.ttfb_ms),
                    elapsed_ms(self.started),
                    usage,
                ));
            }
        }
    }

    async fn finish(&mut self, outcome: Outcome) {
        if let Some(timings) = &mut self.timings {
            timings.finish(outcome);
        }
        let duration_ms = elapsed_ms(self.started);
        for &(event_id, account_id) in &self.events {
            finish_usage_event(
                self.state.clone(),
                event_id,
                account_id,
                (Some(self.ttfb_ms), self.first_token_ms),
                duration_ms,
                self.usage,
            )
            .await;
        }
        if let Some(timings) = &self.timings {
            timings.save(&self.state, &self.events).await;
        }
        self.timings = None;
        self.events.clear();
        self.permits.clear();
    }
}

impl Drop for StreamAccounting {
    fn drop(&mut self) {
        // Dropping a response (including a client disconnect) must finalize its
        // accounting before releasing the next request waiting on this Key.
        if !self.events.is_empty()
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            let state = self.state.clone();
            let events = std::mem::take(&mut self.events);
            let ttfb_ms = self.ttfb_ms;
            let first_token_ms = self.first_token_ms;
            let duration_ms = self.started.elapsed().as_millis().min(i32::MAX as u128) as i32;
            let usage = self.usage;
            let permits = std::mem::take(&mut self.permits);
            let mut timings = self.timings.take();
            if let Some(timings) = &mut timings {
                timings.finish(Outcome::ClientDisconnected);
            }
            runtime.spawn(async move {
                let _permits = permits;
                for &(event_id, account_id) in &events {
                    finish_usage_event(
                        state.clone(),
                        event_id,
                        account_id,
                        (Some(ttfb_ms), first_token_ms),
                        duration_ms,
                        usage,
                    )
                    .await;
                }
                if let Some(timings) = timings {
                    timings.save(&state, &events).await;
                }
            });
        }
    }
}

async fn finish_usage_event(
    state: AppState,
    event_id: Uuid,
    account_id: Uuid,
    (ttfb_ms, first_token_ms): (Option<i32>, Option<i32>),
    duration_ms: i32,
    usage: Option<TokenUsage>,
) {
    update_usage_event(
        state.clone(),
        event_id,
        account_id,
        ttfb_ms,
        duration_ms,
        usage,
    )
    .await;
    if first_token_ms.is_some() {
        let _ =
            sqlx::query("UPDATE usage_events SET first_token_ms=$3 WHERE id=$1 AND account_id=$2")
                .bind(event_id)
                .bind(account_id)
                .bind(first_token_ms)
                .execute(&state.db)
                .await;
    }
    let _ =
        sqlx::query("UPDATE usage_events SET in_flight_until=NULL WHERE id=$1 AND account_id=$2")
            .bind(event_id)
            .bind(account_id)
            .execute(&state.db)
            .await;
}

async fn update_usage_event(
    state: AppState,
    event_id: Uuid,
    account_id: Uuid,
    ttfb_ms: Option<i32>,
    duration_ms: i32,
    usage: Option<TokenUsage>,
) {
    let (input, output, cached) = usage.map_or((None, None, None), |v| {
        (Some(v.input), Some(v.output), v.cached)
    });
    // The request's price snapshot is immutable. A late/out-of-order stream update
    // cannot replace a more complete token count or re-price at today's rates.
    let _=sqlx::query(
        "UPDATE usage_events SET ttfb_ms=COALESCE(ttfb_ms,$1),duration_ms=GREATEST(duration_ms,$2),
         input_tokens=COALESCE($3,input_tokens),output_tokens=COALESCE($4,output_tokens),
         cached_tokens=COALESCE($5,cached_tokens),cache_usage_reported=COALESCE(cache_usage_reported,false) OR $7,
         cost_usd=CASE WHEN price_snapshot IS NOT NULL AND $3::bigint IS NOT NULL AND $4::bigint IS NOT NULL
           THEN (GREATEST($3-LEAST(GREATEST(COALESCE($5,cached_tokens,0),0),GREATEST($3,0)),0)*(price_snapshot->>'input_per_1m')::float8
             + GREATEST($4,0)*(price_snapshot->>'output_per_1m')::float8
             + LEAST(GREATEST(COALESCE($5,cached_tokens,0),0),GREATEST($3,0))*(price_snapshot->>'cached_read_per_1m')::float8)/1000000.0 ELSE cost_usd END,
         pricing_status=CASE WHEN price_snapshot IS NOT NULL AND $3::bigint IS NOT NULL AND $4::bigint IS NOT NULL THEN 'priced' ELSE pricing_status END
         WHERE id=$6 AND account_id=$8 AND ($3::bigint IS NULL OR (COALESCE(input_tokens,0)<= $3 AND COALESCE(output_tokens,0)<= $4))"
    ).bind(ttfb_ms).bind(duration_ms).bind(input).bind(output).bind(cached).bind(event_id).bind(cached.is_some()).bind(account_id)
        .execute(&state.db).await;
}

fn channel_secret(state: &AppState, row: &sqlx::postgres::PgRow) -> Result<String> {
    if !row.try_get::<bool, _>("enabled")? {
        return Err(AppError::BadRequest("渠道已停用，请先启用。".into()));
    }
    let ciphertext = row
        .try_get::<Option<String>, _>("secret_ciphertext")?
        .ok_or_else(|| {
            AppError::BadRequest("未配置可用的上游 API Key，请在编辑渠道中填写。".into())
        })?;
    state
        .crypto
        .decrypt(&ciphertext)
        .map_err(|_| AppError::BadRequest("上游 API Key 无法读取，请在编辑渠道中重新填写。".into()))
}

pub async fn fetch_channel_models(
    state: &AppState,
    account_id: Uuid,
    channel_id: Uuid,
) -> Result<Vec<String>> {
    let row = sqlx::query(
        "SELECT c.enabled,c.protocol,c.base_url,ck.secret_ciphertext FROM channels c \
         LEFT JOIN channel_keys ck ON ck.channel_id=c.id AND ck.enabled=true \
         WHERE c.id=$1 AND c.account_id=$2 \
         ORDER BY ck.created_at,ck.id LIMIT 1",
    )
    .bind(channel_id)
    .bind(account_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    let protocol: String = row
        .try_get("protocol")
        .map_err(|_| AppError::Internal("invalid channel protocol".into()))?;
    let base_url: String = row
        .try_get("base_url")
        .map_err(|_| AppError::Internal("invalid channel URL".into()))?;
    let secret = channel_secret(state, &row)?;
    fetch_models_from_config(state, &protocol, &base_url, &secret).await
}

pub async fn fetch_models_from_config(
    state: &AppState,
    protocol: &str,
    base_url: &str,
    secret: &str,
) -> Result<Vec<String>> {
    let path = if protocol == "gemini" {
        "/v1beta/models"
    } else {
        "/v1/models"
    };
    let target = join_base_url(base_url, path)?;
    ensure_safe_target(&target).await?;
    let mut models = Vec::new();
    // Anthropic and Gemini page their lists; the cap bounds a relay that never
    // stops paging.
    let mut page: Option<String> = None;
    for _ in 0..MODEL_LIST_MAX_PAGES {
        let mut request = state
            .http
            .get(target.clone())
            .timeout(std::time::Duration::from_secs(30));
        request = match protocol {
            "gemini" => {
                let mut request = request
                    .header("x-goog-api-key", secret)
                    .query(&[("pageSize", "1000")]);
                if let Some(page) = &page {
                    request = request.query(&[("pageToken", page)]);
                }
                request
            }
            "anthropic" => {
                let mut request = request
                    .header("x-api-key", secret)
                    .header("anthropic-version", "2023-06-01")
                    .query(&[("limit", "1000")]);
                if let Some(page) = &page {
                    request = request.query(&[("after_id", page)]);
                }
                request
            }
            _ => request.header(header::AUTHORIZATION, format!("Bearer {secret}")),
        };
        let response = request.send().await.map_err(|error| {
            AppError::BadRequest(if error.is_timeout() {
                "获取上游模型列表超时".to_owned()
            } else {
                "无法连接上游获取模型列表".to_owned()
            })
        })?;
        let status = response.status();
        let body = read_limited(response, MAX_METADATA_BODY)
            .await
            .map_err(|error| {
                AppError::BadRequest(match error {
                    BodyError::TooLarge => "上游模型列表过大".into(),
                    BodyError::Read => "读取上游模型列表失败".into(),
                })
            })?;
        if !status.is_success() {
            return Err(AppError::BadRequest(format!(
                "上游模型列表返回 HTTP {}{}",
                status.as_u16(),
                if protocol == "anthropic" && status.as_u16() == 404 {
                    "，该上游可能不支持获取模型列表，请手动填写模型或开启全部转发"
                } else {
                    ""
                }
            )));
        }
        let payload: Value = serde_json::from_slice(&body)
            .map_err(|_| AppError::BadRequest("上游模型列表不是有效的 JSON".into()))?;
        if protocol == "gemini" {
            if let Some(items) = payload.get("models").and_then(Value::as_array) {
                // Embedding and image models cannot serve generateContent.
                for item in items.iter().filter(|item| gemini_generates(item)) {
                    if let Some(name) = item.get("name").and_then(Value::as_str) {
                        models.push(name.strip_prefix("models/").unwrap_or(name).to_owned());
                    }
                }
            }
        } else if let Some(items) = payload.get("data").and_then(Value::as_array) {
            for item in items {
                if let Some(id) = item.get("id").and_then(Value::as_str) {
                    models.push(id.to_owned());
                }
            }
        }
        page = match protocol {
            "gemini" => payload["nextPageToken"].as_str(),
            "anthropic" if payload["has_more"] == true => payload["last_id"].as_str(),
            _ => None,
        }
        .filter(|token| !token.is_empty())
        .map(str::to_owned);
        if page.is_none() {
            break;
        }
    }
    models.retain(|name| model_rules::concrete(name));
    models.sort();
    models.dedup();
    if models.is_empty() {
        return Err(AppError::BadRequest("上游没有返回可用的模型".into()));
    }
    Ok(models)
}

const MODEL_LIST_MAX_PAGES: usize = 10;

/// Relays may omit `supportedGenerationMethods`; their models are kept.
fn gemini_generates(item: &Value) -> bool {
    item.get("supportedGenerationMethods")
        .and_then(Value::as_array)
        .is_none_or(|methods| methods.iter().any(|method| method == "generateContent"))
}

pub async fn probe_channel(
    state: &AppState,
    account_id: Uuid,
    channel_id: Uuid,
) -> Result<(bool, String)> {
    let started_at = Utc::now();
    let channel: Option<(String, bool, Option<String>)> = sqlx::query_as(
        "SELECT kind,enabled,test_model FROM channels WHERE id=$1 AND account_id=$2",
    )
    .bind(channel_id)
    .bind(account_id)
    .fetch_optional(&state.db)
    .await?;
    if let Some((kind, enabled, test_model)) = channel
        && kind == "vair_key"
    {
        if !enabled {
            return Err(AppError::BadRequest("渠道已停用，请先启用。".into()));
        }
        return probe_relay_channel(state, account_id, channel_id, test_model, started_at).await;
    }
    let row = sqlx::query(
        "SELECT c.enabled,c.protocol,c.base_url,c.models,c.model_mapping,c.model_mode,c.test_model,ck.secret_ciphertext,r.upstream_model \
         FROM channels c LEFT JOIN channel_keys ck ON ck.channel_id=c.id AND ck.enabled=true \
         LEFT JOIN LATERAL (SELECT upstream_model FROM model_routes WHERE channel_id=c.id AND account_id=$2 AND enabled=true AND protocol=c.protocol ORDER BY priority LIMIT 1) r ON true \
         WHERE c.id=$1 AND c.account_id=$2 ORDER BY ck.created_at,ck.id LIMIT 1",
    )
    .bind(channel_id)
    .bind(account_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    let protocol: String = row
        .try_get("protocol")
        .map_err(|_| AppError::Internal("invalid channel protocol".into()))?;
    let base_url: String = row
        .try_get("base_url")
        .map_err(|_| AppError::Internal("invalid channel URL".into()))?;
    let secret = channel_secret(state, &row)?;
    let upstream_model: Option<String> = row
        .try_get("upstream_model")
        .map_err(|_| AppError::Internal("invalid model route".into()))?;
    let models: Value = row
        .try_get("models")
        .map_err(|_| AppError::Internal("invalid channel models".into()))?;
    let model_mapping: Value = row
        .try_get("model_mapping")
        .map_err(|_| AppError::Internal("invalid model mapping".into()))?;
    let test_model: Option<String> = row
        .try_get("test_model")
        .map_err(|_| AppError::Internal("invalid channel test model".into()))?;
    let discovered_model = models
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter(|model| model_rules::concrete(model))
                .min()
        })
        .map(|public| {
            model_mapping
                .get(public)
                .and_then(Value::as_str)
                .unwrap_or(public)
                .to_owned()
        });
    let mapped_model = model_mapping
        .as_object()
        .and_then(|items| items.values().next())
        .and_then(Value::as_str)
        .map(str::to_owned);
    // Real requests show which upstream model the channel serves; the first
    // configured model by name is often the oldest, and the first retired.
    let recent_model: Option<String> = sqlx::query_scalar(
        "SELECT upstream_model FROM usage_events
         WHERE account_id=$1 AND channel_id=$2 AND status BETWEEN 200 AND 299
           AND upstream_model IS NOT NULL AND created_at>now()-interval '7 days'
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(account_id)
    .bind(channel_id)
    .fetch_optional(&state.db)
    .await?;
    let test_model = test_model.filter(|value| model_rules::concrete(value));
    let explicit = test_model.is_some();
    let Some(mut model) = test_model
        .or(recent_model.clone())
        .or(discovered_model)
        .or(mapped_model.filter(|model| model_rules::concrete(model)))
        .or(upstream_model.filter(|model| model_rules::concrete(model)))
    else {
        return Err(AppError::BadRequest(
            "需要配置具体测试模型；本次未发起检查，保留上次健康状态".into(),
        ));
    };
    let mut outcome = probe_request(state, &protocol, &base_url, &secret, &model).await?;
    let mut note = None;
    // A chosen test model the upstream no longer serves says nothing about the
    // channel, so the check falls back once to a model real requests used.
    if outcome.kind == CheckKind::ModelUnavailable
        && explicit
        && let Some(recent) = recent_model.filter(|recent| *recent != model)
    {
        outcome = probe_request(state, &protocol, &base_url, &secret, &recent).await?;
        note = Some(format!(
            "测试模型 {model} 不可用，已改用 {recent} 检查。建议更新测试模型。"
        ));
        model = recent;
    }
    let result = (
        outcome.kind == CheckKind::Healthy,
        outcome.message,
        outcome.ttfb_ms,
    );
    record_availability(
        state,
        channel_id,
        Some(&model),
        Some(outcome.kind),
        &result,
        started_at,
    )
    .await?;
    sqlx::query("UPDATE channel_health SET check_note=$2 WHERE channel_id=$1")
        .bind(channel_id)
        .bind(&note)
        .execute(&state.db)
        .await?;
    Ok((result.0, result.1))
}

/// Checks a VAIR key channel through its source key. The probe is subject to
/// the key's permissions, channel scope and limits, and counts toward them as
/// monitor usage.
async fn probe_relay_channel(
    state: &AppState,
    account_id: Uuid,
    channel_id: Uuid,
    test_model: Option<String>,
    started_at: DateTime<Utc>,
) -> Result<(bool, String)> {
    let model = match test_model.filter(|value| model_rules::concrete(value)) {
        Some(model) => Some(model),
        None => match load_relay_channels(state, account_id)
            .await?
            .into_iter()
            .find(|relay| relay.channel_id == channel_id)
        {
            Some(relay) => relay_check_model(state, account_id, &relay.source).await?,
            None => None,
        },
    };
    let target = match &model {
        Some(model) => relay_probe_target(state, account_id, channel_id, model).await?,
        None => Err("来源 Key 当前没有可用模型".to_owned()),
    };
    let (status, result) = match target {
        Ok(target) => {
            let outcome = probe_request(
                state,
                &target.protocol,
                &target.base_url,
                &target.secret,
                &target.model,
            )
            .await?;
            target
                .record(
                    state,
                    outcome.status,
                    (outcome.ttfb_ms, outcome.duration_ms),
                    outcome.usage,
                )
                .await;
            (
                Some(outcome.kind),
                (
                    outcome.kind == CheckKind::Healthy,
                    outcome.message,
                    outcome.ttfb_ms,
                ),
            )
        }
        // The probe never left the gateway: the source key is paused, at a limit
        // or lacks the model. Routing re-checks the key on every request, so this
        // must not mark the channel unhealthy and keep it out after the key recovers.
        Err(reason) => (None, (false, reason, None)),
    };
    record_availability(
        state,
        channel_id,
        model.as_deref(),
        status,
        &result,
        started_at,
    )
    .await?;
    Ok((result.0, result.1))
}

/// Stores a check result: what the upstream's response said, or `None` when
/// the check failed without reaching the upstream, so routing is unaffected.
async fn record_availability(
    state: &AppState,
    channel_id: Uuid,
    model: Option<&str>,
    kind: Option<CheckKind>,
    result: &(bool, String, Option<i32>),
    started_at: DateTime<Utc>,
) -> Result<()> {
    let finished_at = Utc::now();
    let status = kind.map_or("unknown", CheckKind::status);
    let health = match kind {
        Some(CheckKind::Healthy) => Health::Success,
        Some(kind @ (CheckKind::RateLimited | CheckKind::ModelUnavailable)) => {
            Health::Uncounted(kind.status(), &result.1)
        }
        Some(kind @ (CheckKind::Credential | CheckKind::Quota)) => {
            Health::Rejected(&result.1, kind.error_kind().unwrap_or_default())
        }
        Some(CheckKind::Failed) => Health::Failure(&result.1),
        None => Health::Unreached(&result.1),
    };
    update_health(state, channel_id, health, Source::Check).await?;
    // Health checks are also monitoring runs. Keep this best-effort so an
    // observability write cannot make a valid channel check fail.
    let latency_ms = (finished_at - started_at)
        .num_milliseconds()
        .clamp(0, i32::MAX as i64) as i32;
    let _ = sqlx::query(
        "INSERT INTO channel_monitor_runs
            (channel_id,kind,model,status,latency_ms,ttfb_ms,error_code,details,started_at,finished_at)
         VALUES ($1,'availability',$2,$3,$4,$5,$6,$7,$8,$9)",
    )
    .bind(channel_id)
    .bind(model)
    .bind(status)
    .bind(latency_ms)
    .bind(result.2)
    .bind(if result.0 {
        None::<String>
    } else {
        Some(result.1.clone())
    })
    .bind(json!({"message": result.1}))
    .bind(started_at)
    .bind(finished_at)
    .execute(&state.db)
    .await;
    Ok(())
}

/// Least time until the next check while failures are counted. Later checks
/// wait as long as the channel has been failing, at most the regular interval.
const RECHECK_AFTER: chrono::Duration = chrono::Duration::minutes(10);

enum Health<'a> {
    Success,
    Failure(&'a str),
    /// A check the upstream refused for the credential or balance, with
    /// `CheckKind::error_kind`. Counted like any failure, but alerted without
    /// waiting, since it will not clear by itself.
    Rejected(&'a str, &'static str),
    /// A check that reached the upstream but says nothing about the channel's
    /// availability, with what it saw ('rate_limited', 'model_unavailable'):
    /// shown, but neither counted nor changing the verdict.
    Uncounted(&'static str, &'a str),
    /// A check that never reached the upstream: shown like `Uncounted`.
    Unreached(&'a str),
}

/// Whether an availability result comes from a scheduled check or a real request.
#[derive(Clone, Copy, PartialEq)]
enum Source {
    Check,
    Traffic,
}

/// Least time an unavailable channel stays out of routing without a counted
/// failure, while checks cannot conclude; twice the check interval if longer.
const VERDICT_KEPT_FOR: chrono::Duration = chrono::Duration::hours(1);
/// How long a channel stays unavailable before scheduled checks email about it.
const ALERT_AFTER: chrono::Duration = chrono::Duration::minutes(30);
/// Least time between two unavailability emails for the same channel.
const ALERT_COOLDOWN: chrono::Duration = chrono::Duration::hours(24);

#[derive(Debug, PartialEq)]
enum AvailabilityEmail {
    Unavailable,
    Recovered,
}

/// The email, if any, for a channel now in `status`. Routing already moves
/// unavailable channels aside, so only failures someone has to act on are sent:
/// a refused credential or balance, from a check or a real request, or
/// unavailability that scheduled checks still see after `ALERT_AFTER`. Other
/// real request failures change routing but never send email on their own. A
/// recovery email follows only an unavailability email.
fn availability_email(
    status: &str,
    rejected: bool,
    source: Source,
    alert_open: bool,
    down_for: Option<chrono::Duration>,
    since_last_alert: Option<chrono::Duration>,
) -> Option<AvailabilityEmail> {
    match status {
        "healthy" if alert_open => Some(AvailabilityEmail::Recovered),
        "unhealthy"
            if !alert_open
                && since_last_alert.is_none_or(|elapsed| elapsed >= ALERT_COOLDOWN)
                && (rejected
                    || source == Source::Check
                        && down_for.is_some_and(|down| down >= ALERT_AFTER)) =>
        {
            Some(AvailabilityEmail::Unavailable)
        }
        _ => None,
    }
}

/// Applies one check or real request to the channel's availability. Failures
/// count toward the channel's threshold; any success resets the count. While
/// failures are counted the next check backs off from `RECHECK_AFTER`, and an
/// unavailable channel is checked again once `ALERT_AFTER` has passed so the
/// alert is not delayed. Emails follow `availability_email`.
async fn update_health(
    state: &AppState,
    channel_id: Uuid,
    health: Health<'_>,
    source: Source,
) -> Result<()> {
    let mut tx = state.db.begin().await?;
    sqlx::query("INSERT INTO channel_health(channel_id,status) VALUES ($1,'unknown') ON CONFLICT(channel_id) DO NOTHING")
        .bind(channel_id)
        .execute(&mut *tx)
        .await?;
    let row = sqlx::query(
        "SELECT h.status,h.consecutive_failures,h.alert_open,c.account_id,c.name,
           h.last_failure_at IS NULL OR h.last_failure_at<now()-GREATEST(
             make_interval(secs => 2*COALESCE(mc.availability_interval_seconds,1800)),make_interval(secs => $2)) AS verdict_stale,
           EXTRACT(EPOCH FROM now()-h.unhealthy_since)::float8 AS down_for,
           EXTRACT(EPOCH FROM now()-h.last_alert_at)::float8 AS since_last_alert,
           COALESCE(mc.availability_failure_threshold,2) AS threshold,
           COALESCE(mc.availability_route_policy,'exclude') AS policy
         FROM channel_health h JOIN channels c ON c.id=h.channel_id
         LEFT JOIN channel_monitor_configs mc ON mc.channel_id=c.id
         WHERE h.channel_id=$1 FOR UPDATE OF h",
    )
    .bind(channel_id)
    .bind(VERDICT_KEPT_FOR.num_seconds() as f64)
    .fetch_one(&mut *tx)
    .await?;
    let previous: String = row.try_get("status")?;
    let failures: i32 = row.try_get("consecutive_failures")?;
    let threshold: i32 = row.try_get("threshold")?;
    let alert_open: bool = row.try_get("alert_open")?;
    let interval = |column| -> Result<Option<chrono::Duration>> {
        Ok(row
            .try_get::<Option<f64>, _>(column)?
            .map(|seconds| chrono::Duration::milliseconds((seconds * 1000.0) as i64)))
    };
    let (down_for, since_last_alert) = (interval("down_for")?, interval("since_last_alert")?);
    let stale = previous == "unhealthy" && row.try_get::<bool, _>("verdict_stale")?;
    // A check without a verdict keeps the previous one, unless an unavailable
    // channel has had no counted failure for long: it then routes normally
    // again, so a check that can never conclude does not keep it out for good.
    let inconclusive = |observation: &'static str, reason| {
        if stale {
            (
                "unknown".to_owned(),
                0,
                Some(reason),
                false,
                Some(observation),
            )
        } else {
            (
                previous.clone(),
                failures,
                Some(reason),
                false,
                Some(observation),
            )
        }
    };
    let (status, failures, error, rejected, observation) = match health {
        Health::Success => ("healthy".to_owned(), 0, None, false, None),
        Health::Failure(reason) | Health::Rejected(reason, _) => {
            let failures = failures.saturating_add(1);
            let status = if failures >= threshold {
                "unhealthy".to_owned()
            } else {
                previous.clone()
            };
            (
                status,
                failures,
                Some(reason),
                matches!(health, Health::Rejected(..)),
                None,
            )
        }
        Health::Uncounted(observation, reason) => inconclusive(observation, reason),
        Health::Unreached(reason) => inconclusive("unknown", reason),
    };
    let error_kind = match health {
        Health::Rejected(_, kind) => Some(kind),
        _ => None,
    };
    let email = availability_email(
        &status,
        rejected,
        source,
        alert_open,
        down_for,
        since_last_alert,
    );
    // A check without a verdict keeps the outage's start, its cause, what to
    // fix and any open alert; an expired verdict ends the outage but keeps the alert, so
    // the next success still sends the recovery email.
    sqlx::query(
        "UPDATE channel_health SET status=$2,consecutive_failures=$3,checked_at=now(),updated_at=now(),
           last_error=CASE WHEN $7 IS NOT NULL AND $2='unhealthy' THEN last_error ELSE $4 END,
           last_observation=$7,last_error_kind=CASE WHEN $7 IS NULL THEN $6 ELSE last_error_kind END,
           last_failure_at=CASE WHEN $7 IS NULL AND $3>0 THEN now() WHEN $3>0 THEN last_failure_at END,
           unhealthy_since=CASE WHEN $2='unhealthy' THEN COALESCE(unhealthy_since,now()) END,
           failing_since=CASE WHEN $3>0 THEN COALESCE(failing_since,now()) END,
           alert_open=CASE $5 WHEN 'unavailable' THEN true WHEN 'recovered' THEN false ELSE alert_open END,
           last_alert_at=CASE WHEN $5='unavailable' THEN now() ELSE last_alert_at END
         WHERE channel_id=$1",
    )
    .bind(channel_id)
    .bind(&status)
    .bind(failures)
    .bind(error)
    .bind(match email {
        Some(AvailabilityEmail::Unavailable) => Some("unavailable"),
        Some(AvailabilityEmail::Recovered) => Some("recovered"),
        None => None,
    })
    .bind(error_kind)
    .bind(observation)
    .execute(&mut *tx)
    .await?;
    // LEAST skips the alert deadline when it is NULL, i.e. already passed or not pending.
    sqlx::query(
        "UPDATE channel_monitor_configs mc SET next_availability_at=now() + CASE WHEN h.consecutive_failures>0 THEN LEAST(
           make_interval(secs => mc.availability_interval_seconds),
           GREATEST(make_interval(secs => $2), now()-h.failing_since),
           CASE WHEN h.status='unhealthy' AND NOT h.alert_open AND h.unhealthy_since+make_interval(secs => $3)>now()
             THEN h.unhealthy_since+make_interval(secs => $3)-now() END)
         ELSE make_interval(secs => mc.availability_interval_seconds) END,
         updated_at=now() FROM channel_health h WHERE mc.channel_id=$1 AND h.channel_id=mc.channel_id",
    )
    .bind(channel_id)
    .bind(RECHECK_AFTER.num_seconds() as f64)
    .bind(ALERT_AFTER.num_seconds() as f64)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    if let Some(email) = email {
        let name: String = row.try_get("name")?;
        let routed = if row.try_get::<String, _>("policy")? == "observe" {
            "按设置仅记录和告警，路由不受影响。"
        } else {
            "已移到路由最后，只在其他渠道都不可用时才会尝试。"
        };
        let (subject, body) = match email {
            AvailabilityEmail::Unavailable => (
                "渠道可用性告警",
                format!(
                    "渠道「{name}」{}，已连续失败 {failures} 次。\n最近一次失败：{}\n{routed}\n\n恢复后会再通知你。可在 {PRODUCT}「渠道监控 → 可用性监控」中查看。",
                    if rejected {
                        "被上游拒绝，请检查渠道密钥或余额"
                    } else {
                        "持续不可用超过 30 分钟"
                    },
                    error.unwrap_or("未知原因")
                ),
            ),
            AvailabilityEmail::Recovered => (
                "渠道已恢复",
                format!("渠道「{name}」已恢复可用，路由恢复正常。"),
            ),
        };
        crate::email::account_alert(
            state,
            row.try_get("account_id")?,
            crate::email::AlertKind::Availability,
            subject,
            body,
        )
        .await?;
    }
    Ok(())
}

/// Why a real request counts against its channel, with `CheckKind::error_kind`
/// when the upstream refused the credential or balance.
type TrafficFailure = (String, Option<&'static str>);

/// Counts a real request toward the channel's availability, in the background.
/// Successes are only written when they clear failures.
fn observe_traffic(
    state: &AppState,
    channel_id: Uuid,
    health: ChannelHealth,
    failure: Option<TrafficFailure>,
) {
    if !health.monitored || (failure.is_none() && health.failures == 0 && !health.unhealthy) {
        return;
    }
    let state = state.clone();
    tokio::spawn(async move {
        let outcome = match &failure {
            Some((reason, Some(kind))) => Health::Rejected(reason, kind),
            Some((reason, None)) => Health::Failure(reason),
            None => Health::Success,
        };
        if update_health(&state, channel_id, outcome, Source::Traffic)
            .await
            .is_err()
        {
            eprintln!("channel availability update failed for {channel_id}");
        }
    });
}

/// Where a monitoring probe is sent. Probes of a VAIR key channel go to an
/// upstream reached through the source key and any further hops, holding each
/// hop's concurrency permit until dropped.
pub(crate) struct ProbeTarget {
    pub protocol: String,
    pub base_url: String,
    pub secret: String,
    pub model: String,
    relay: Option<(RelayChain, Uuid)>,
}

impl ProbeTarget {
    /// Records the probe as monitor usage of every source key on the path,
    /// innermost first and linked like a relayed request, priced like any
    /// request, so it counts toward each key's token and cost caps.
    pub(crate) async fn record(
        &self,
        state: &AppState,
        status: u16,
        timing: (Option<i32>, i32),
        usage: Option<TokenUsage>,
    ) {
        let Some((chain, upstream_channel)) = &self.relay else {
            return;
        };
        let Ok(prices) = state.prices().await else {
            return;
        };
        let mut relayed_event_id = None;
        for &(relay, channel) in chain.served(*upstream_channel).iter().rev() {
            relayed_event_id = record_usage(
                state,
                &relay.source,
                UsageContext {
                    public_model: &relay.model,
                    protocol: &self.protocol,
                    upstream_model: Some(&self.model),
                    price: pricing::lookup(&prices, &self.model),
                    in_flight: false,
                    relayed_event_id,
                    source: "monitor",
                    stream: false,
                    input_chars: None,
                    input_shapes: None,
                },
                (status, Some(channel)),
                timing,
                usage,
            )
            .await;
        }
    }
}

/// Resolves a probe of `model` on a VAIR key channel. The source key's model
/// permissions, channel scope, unknown-cost policy, rate, concurrency and spend
/// caps apply; an inner `Err` is the user-facing reason the probe could not run.
pub(crate) async fn relay_probe_target(
    state: &AppState,
    account_id: Uuid,
    channel_id: Uuid,
    model: &str,
) -> Result<std::result::Result<ProbeTarget, String>> {
    let Some(relay) = load_relay_channels(state, account_id)
        .await?
        .into_iter()
        .find(|relay| relay.channel_id == channel_id)
    else {
        return Ok(Err("来源 Key 已停用或已撤销".into()));
    };
    let model = relay
        .model_mapping
        .get(model)
        .and_then(Value::as_str)
        .unwrap_or(model)
        .to_owned();
    let unavailable = || Ok(Err(format!("来源 Key 不能使用模型 {model}")));
    let limited = || Ok(Err("来源 Key 已达到请求频率、并发或费用额度上限".into()));
    // Find the protocol that serves the model before consuming any limits.
    let mut protocol = None;
    for candidate in ["openai", "anthropic", "gemini"] {
        let mut budget = MAX_RELAY_EXPANSIONS;
        let path = [account_id, relay.source.account_id];
        if reachable(state, &relay.source, candidate, &model, &path, &mut budget).await? {
            protocol = Some(candidate);
            break;
        }
    }
    let Some(protocol) = protocol else {
        return unavailable();
    };
    let mut chain = RelayChain::new(account_id);
    match chain
        .enter(state, protocol, channel_id, relay.source, model.clone())
        .await?
    {
        Ok(()) => {}
        Err((StatusCode::TOO_MANY_REQUESTS, _)) => return limited(),
        Err(_) => return unavailable(),
    }
    // Why no hop yielded an upstream, for when none does.
    let mut was_limited = false;
    let candidate = loop {
        match chain.next(state, protocol).await? {
            RelayStep::Candidate(candidate) => {
                if !model_rules::concrete(&candidate.upstream_model) {
                    continue;
                }
                break candidate;
            }
            RelayStep::Unavailable(status, _) => {
                was_limited |= status == StatusCode::TOO_MANY_REQUESTS;
            }
            RelayStep::Exhausted(_) => {}
            RelayStep::Done if was_limited => return limited(),
            RelayStep::Done => return unavailable(),
        }
    };
    Ok(Ok(ProbeTarget {
        protocol: candidate.protocol,
        base_url: candidate.base_url,
        secret: candidate.secret,
        model: candidate.upstream_model,
        relay: Some((chain, candidate.channel_id)),
    }))
}

/// What a check's response says about the channel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum CheckKind {
    Healthy,
    /// 429 without a quota error: the address and key work, the upstream is
    /// only busy, so it neither counts as a failure nor keeps the channel out.
    RateLimited,
    /// 401 or 403: the key needs replacing.
    Credential,
    /// 402, or a quota error behind 429/400: the balance needs topping up.
    Quota,
    /// 400/404 naming the model: the test model is wrong, not the channel, so
    /// like a rate limit it neither counts nor keeps the channel out.
    ModelUnavailable,
    Failed,
}

impl CheckKind {
    fn classify(status: StatusCode, body: &[u8]) -> Self {
        match status.as_u16() {
            200..=299 => Self::Healthy,
            401 | 403 => Self::Credential,
            402 => Self::Quota,
            429 if quota_exhausted(body) => Self::Quota,
            429 => Self::RateLimited,
            400 if quota_exhausted(body) => Self::Quota,
            _ if model_not_supported(status, body) => Self::ModelUnavailable,
            _ => Self::Failed,
        }
    }

    /// Stored as the channel's health status and the run's status.
    pub(crate) fn status(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::RateLimited => "rate_limited",
            Self::ModelUnavailable => "model_unavailable",
            _ => "unhealthy",
        }
    }

    /// Whether a real request tries the next channel. Only errors in the
    /// request itself go back to the client.
    fn fails_over(self, status: StatusCode) -> bool {
        match self {
            Self::Healthy => false,
            Self::Failed => status == StatusCode::REQUEST_TIMEOUT || status.is_server_error(),
            _ => true,
        }
    }

    /// How a real request's error counts toward the channel's availability:
    /// refusals of the credential or balance like a failed check, server
    /// errors as failures, the rest not at all.
    fn traffic_failure(self, status: StatusCode) -> Option<TrafficFailure> {
        match self {
            Self::Credential | Self::Quota => Some((self.message(status, ""), self.error_kind())),
            Self::Failed if status.is_server_error() => {
                Some((format!("上游返回 HTTP {}", status.as_u16()), None))
            }
            _ => None,
        }
    }

    /// Stored with the health so the console can say what to fix.
    fn error_kind(self) -> Option<&'static str> {
        match self {
            Self::Credential => Some("credential"),
            Self::Quota => Some("quota"),
            _ => None,
        }
    }

    fn message(self, status: StatusCode, model: &str) -> String {
        let code = status.as_u16();
        match self {
            Self::Healthy => "检查通过".to_owned(),
            Self::RateLimited => format!("上游限流（HTTP {code}），本次不计入失败"),
            Self::Credential => format!("上游拒绝了 API Key（HTTP {code}）"),
            Self::Quota => format!("上游额度不足（HTTP {code}）"),
            Self::ModelUnavailable => {
                format!("测试模型 {model} 在上游不可用（HTTP {code}），请更换测试模型")
            }
            Self::Failed => format!("上游返回 HTTP {code}"),
        }
    }
}

/// OpenAI reports an empty balance as 429 `insufficient_quota`, Anthropic as a
/// 400 about the credit balance.
fn quota_exhausted(body: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return false;
    };
    let error = &value["error"];
    ["code", "type"]
        .iter()
        .any(|field| error[field] == "insufficient_quota")
        || error["message"]
            .as_str()
            .is_some_and(|message| message.contains("credit balance"))
}

struct ProbeOutcome {
    kind: CheckKind,
    message: String,
    status: u16,
    ttfb_ms: Option<i32>,
    duration_ms: i32,
    usage: Option<TokenUsage>,
}

pub async fn probe_channel_config(
    state: &AppState,
    protocol: &str,
    base_url: &str,
    secret: &str,
    model: &str,
) -> Result<(CheckKind, String, Option<i32>)> {
    let outcome = probe_request(state, protocol, base_url, secret, model).await?;
    Ok((outcome.kind, outcome.message, outcome.ttfb_ms))
}

const PROBE_TIMEOUT_SECONDS: u64 = 30;

async fn probe_request(
    state: &AppState,
    protocol: &str,
    base_url: &str,
    secret: &str,
    model: &str,
) -> Result<ProbeOutcome> {
    if !model_rules::concrete(model) {
        return Err(AppError::BadRequest("测试模型必须是具体名称".into()));
    }
    let (path, body) = match protocol {
        "openai" => (
            "/v1/chat/completions".to_owned(),
            json!({"model":model,"messages":[{"role":"user","content":"health"}],"max_tokens":1}),
        ),
        "anthropic" => (
            "/v1/messages".to_owned(),
            json!({"model":model,"max_tokens":1,"messages":[{"role":"user","content":"health"}]}),
        ),
        "gemini" => (
            format!("/v1beta/models/{model}:generateContent"),
            json!({"contents":[{"parts":[{"text":"health"}]}],"generationConfig":{"maxOutputTokens":1}}),
        ),
        _ => return Err(AppError::BadRequest("unsupported channel protocol".into())),
    };
    let target = join_base_url(base_url, &path)?;
    ensure_safe_target(&target).await?;
    let mut request = state
        .http
        .post(target)
        .timeout(std::time::Duration::from_secs(PROBE_TIMEOUT_SECONDS))
        .header("content-type", "application/json")
        .body(
            serde_json::to_vec(&body)
                .map_err(|_| AppError::Internal("health request serialization failed".into()))?,
        );
    request = match protocol {
        "anthropic" => request
            .header("x-api-key", secret)
            .header("anthropic-version", "2023-06-01"),
        "gemini" => request.header("x-goog-api-key", secret),
        _ => request.header(header::AUTHORIZATION, format!("Bearer {secret}")),
    };
    let started = std::time::Instant::now();
    let elapsed = || started.elapsed().as_millis().min(i32::MAX as u128) as i32;
    Ok(match request.send().await {
        Ok(response) => {
            let ttfb_ms = elapsed();
            let status = response.status();
            let body = read_limited(response, MAX_METADATA_BODY)
                .await
                .unwrap_or_default();
            let kind = CheckKind::classify(status, &body);
            // Token usage lets probes through a source key count toward its caps.
            let usage = if status.is_success() {
                parse_usage(&body)
            } else {
                None
            };
            ProbeOutcome {
                kind,
                message: kind.message(status, model),
                status: status.as_u16(),
                ttfb_ms: Some(ttfb_ms),
                duration_ms: elapsed(),
                usage,
            }
        }
        Err(error) => ProbeOutcome {
            kind: CheckKind::Failed,
            // Relay recipients can read probe errors, but not the source URL.
            message: if error.is_timeout() {
                format!("上游 {PROBE_TIMEOUT_SECONDS} 秒内未响应（超时）")
            } else {
                format!("upstream request failed: {}", error.without_url())
            },
            status: StatusCode::BAD_GATEWAY.as_u16(),
            ttfb_ms: None,
            duration_ms: elapsed(),
            usage: None,
        },
    })
}

#[cfg(test)]
pub(crate) async fn quality_test_candidates(
    state: &AppState,
    account_id: Uuid,
    protocol: &str,
    model: &str,
) -> Result<Vec<Uuid>> {
    Ok(load_candidates(state, account_id, protocol, model, None)
        .await?
        .into_iter()
        .map(|c| c.channel_id)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_emails_only_need_action() {
        use AvailabilityEmail::*;
        let minutes = chrono::Duration::minutes;
        let email = |status, rejected, source, open, down, last| {
            availability_email(status, rejected, source, open, down, last)
        };
        // Short outages and real requests never email.
        assert_eq!(
            email(
                "unhealthy",
                false,
                Source::Check,
                false,
                Some(minutes(5)),
                None
            ),
            None
        );
        assert_eq!(
            email(
                "unhealthy",
                false,
                Source::Traffic,
                false,
                Some(minutes(90)),
                None
            ),
            None
        );
        assert_eq!(
            email("unhealthy", false, Source::Check, false, None, None),
            None
        );
        // Sustained outages and refused credentials do.
        assert_eq!(
            email(
                "unhealthy",
                false,
                Source::Check,
                false,
                Some(minutes(30)),
                None
            ),
            Some(Unavailable)
        );
        assert_eq!(
            email("unhealthy", true, Source::Check, false, None, None),
            Some(Unavailable)
        );
        // Once per outage, and at most once a day per channel.
        assert_eq!(
            email(
                "unhealthy",
                true,
                Source::Check,
                true,
                Some(minutes(90)),
                None
            ),
            None
        );
        assert_eq!(
            email(
                "unhealthy",
                true,
                Source::Check,
                false,
                None,
                Some(minutes(600))
            ),
            None
        );
        assert_eq!(
            email(
                "unhealthy",
                true,
                Source::Check,
                false,
                None,
                Some(minutes(24 * 60))
            ),
            Some(Unavailable)
        );
        // Recovery follows an alert, from either source; flaps stay silent.
        assert_eq!(
            email("healthy", false, Source::Traffic, true, None, None),
            Some(Recovered)
        );
        assert_eq!(
            email("healthy", false, Source::Check, false, None, None),
            None
        );
        assert_eq!(
            email(
                "unknown",
                false,
                Source::Check,
                true,
                Some(minutes(90)),
                None
            ),
            None
        );
    }

    #[test]
    fn gemini_model_lists_keep_only_generating_models() {
        assert!(gemini_generates(
            &json!({"supportedGenerationMethods": ["generateContent", "countTokens"]})
        ));
        assert!(!gemini_generates(
            &json!({"supportedGenerationMethods": ["embedContent"]})
        ));
        assert!(gemini_generates(&json!({"name": "models/relay-model"})));
    }

    #[test]
    fn check_kinds_separate_rate_limits_from_failures() {
        let kind = |status: u16, body: &str| {
            CheckKind::classify(StatusCode::from_u16(status).unwrap(), body.as_bytes())
        };
        assert_eq!(kind(200, ""), CheckKind::Healthy);
        assert_eq!(
            kind(429, r#"{"error":{"type":"rate_limit_error"}}"#),
            CheckKind::RateLimited
        );
        assert_eq!(
            kind(429, r#"{"error":{"code":"insufficient_quota"}}"#),
            CheckKind::Quota
        );
        assert_eq!(
            kind(
                400,
                r#"{"error":{"message":"Your credit balance is too low"}}"#
            ),
            CheckKind::Quota
        );
        assert_eq!(kind(402, ""), CheckKind::Quota);
        assert_eq!(kind(401, ""), CheckKind::Credential);
        assert_eq!(kind(403, ""), CheckKind::Credential);
        assert_eq!(kind(400, "{}"), CheckKind::Failed);
        assert_eq!(
            kind(404, r#"{"error":{"code":"model_not_found"}}"#),
            CheckKind::ModelUnavailable
        );
        assert_eq!(kind(404, ""), CheckKind::Failed);
        assert_eq!(kind(502, ""), CheckKind::Failed);
        assert_eq!(CheckKind::RateLimited.status(), "rate_limited");
        assert_eq!(CheckKind::Quota.status(), "unhealthy");
    }

    #[tokio::test]
    async fn compressed_upstream_streams_are_decoded_before_usage_parsing() {
        use std::io::Write;
        assert!(!forwards_client_header(&header::ACCEPT_ENCODING));
        assert!(forwards_client_header(&HeaderName::from_static(
            "anthropic-version"
        )));
        let sse = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12,\"output_tokens\":1}}}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":8}}\n\n";
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(sse.as_bytes()).unwrap();
        let compressed = encoder.finish().unwrap();
        // Like some relays, the mock compresses whenever the request accepts gzip.
        let app = axum::Router::new().fallback(move |headers: HeaderMap| {
            let compressed = compressed.clone();
            async move {
                let accepts_gzip = headers
                    .get(header::ACCEPT_ENCODING)
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| value.contains("gzip"));
                assert!(accepts_gzip);
                (
                    [
                        (header::CONTENT_TYPE, "text/event-stream"),
                        (header::CONTENT_ENCODING, "gzip"),
                    ],
                    compressed,
                )
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1/messages", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let response = reqwest::Client::new().post(url).send().await.unwrap();
        assert!(response.headers().get(header::CONTENT_ENCODING).is_none());
        let body = response.bytes().await.unwrap();
        assert_eq!(body, sse.as_bytes());
        assert_eq!(
            parse_stream_usage(&body),
            Some(TokenUsage {
                input: 12,
                output: 8,
                cached: None
            })
        );
        server.abort();
    }

    #[test]
    fn parses_openai_usage() {
        let usage = parse_usage(br#"{"usage":{"prompt_tokens":12,"completion_tokens":8,"prompt_tokens_details":{"cached_tokens":3}}}"#);
        assert_eq!(
            usage,
            Some(TokenUsage {
                input: 12,
                output: 8,
                cached: Some(3)
            })
        );
    }

    #[test]
    fn parses_domestic_openai_compatible_cache_usage() {
        // DeepSeek reports hits both nested and at the top level.
        assert_eq!(
            parse_usage(br#"{"usage":{"prompt_tokens":12,"completion_tokens":8,"total_tokens":20,"prompt_cache_hit_tokens":3,"prompt_cache_miss_tokens":9,"prompt_tokens_details":{"cached_tokens":3}}}"#),
            Some(TokenUsage { input: 12, output: 8, cached: Some(3) })
        );
        assert_eq!(
            parse_usage(br#"{"usage":{"prompt_tokens":12,"completion_tokens":8,"prompt_cache_hit_tokens":3,"prompt_cache_miss_tokens":9}}"#),
            Some(TokenUsage { input: 12, output: 8, cached: Some(3) })
        );
        // Kimi: hits and cache writes are both part of `prompt_tokens`.
        assert_eq!(
            parse_usage(br#"{"usage":{"prompt_tokens":12,"completion_tokens":8,"prompt_tokens_details":{"cached_tokens":3,"cache_write_tokens":4}}}"#),
            Some(TokenUsage { input: 12, output: 8, cached: Some(3) })
        );
        assert_eq!(
            parse_usage(
                br#"{"usage":{"prompt_tokens":12,"completion_tokens":8,"cached_tokens":3}}"#
            ),
            Some(TokenUsage {
                input: 12,
                output: 8,
                cached: Some(3)
            })
        );
    }

    #[test]
    fn null_usage_is_missing() {
        assert_eq!(parse_usage(br#"{"usage":null}"#), None);
        assert_eq!(
            parse_usage(br#"{"usage":null,"usageMetadata":{"promptTokenCount":12,"candidatesTokenCount":8}}"#),
            Some(TokenUsage { input: 12, output: 8, cached: Some(0) })
        );
        let body = b"data: {\"choices\":[],\"usage\":null}\n\ndata: [DONE]\n";
        assert_eq!(parse_stream_usage(body), None);
    }

    #[test]
    fn parses_anthropic_and_gemini_usage() {
        assert_eq!(
            parse_usage(
                br#"{"usage":{"input_tokens":12,"output_tokens":8,"cache_read_input_tokens":3}}"#
            ),
            Some(TokenUsage {
                input: 15,
                output: 8,
                cached: Some(3)
            })
        );
        assert_eq!(
            parse_usage(
                br#"{"usage":{"input_tokens":12,"output_tokens":8,"cache_creation_input_tokens":4,"cache_read_input_tokens":3}}"#
            ),
            Some(TokenUsage { input: 19, output: 8, cached: Some(3) })
        );
        assert_eq!(
            parse_usage(br#"{"usageMetadata":{"promptTokenCount":12,"candidatesTokenCount":8,"cachedContentTokenCount":3}}"#),
            Some(TokenUsage { input: 12, output: 8, cached: Some(3) })
        );
    }

    #[test]
    fn parses_stream_usage_events() {
        let body = br#"data: {"choices": [{"delta": {}}]}
data: {"usage":{"prompt_tokens":12,"completion_tokens":8}}
data: [DONE]
"#;
        assert_eq!(
            parse_stream_usage(body),
            Some(TokenUsage {
                input: 12,
                output: 8,
                cached: None
            })
        );
    }

    #[test]
    fn distinguishes_reported_zero_from_missing_cache_usage() {
        for body in [
            br#"{"usage":{"prompt_tokens":12,"completion_tokens":8,"prompt_tokens_details":{"cached_tokens":0}}}"#.as_slice(),
            br#"{"response":{"usage":{"input_tokens":12,"output_tokens":8,"input_tokens_details":{"cached_tokens":0}}}}"#,
            br#"{"usage":{"input_tokens":12,"output_tokens":8,"cache_read_input_tokens":0}}"#,
            br#"{"usageMetadata":{"promptTokenCount":12,"candidatesTokenCount":8,"cachedContentTokenCount":0}}"#,
        ] {
            assert_eq!(parse_usage(body), Some(TokenUsage { input: 12, output: 8, cached: Some(0) }));
        }
        for body in [
            br#"{"usage":{"input_tokens":12,"output_tokens":8}}"#.as_slice(),
            br#"{"usage":{"input_tokens":12,"output_tokens":8,"input_tokens_details":{"cached_tokens":null}}}"#,
        ] {
            assert_eq!(parse_usage(body), Some(TokenUsage { input: 12, output: 8, cached: None }));
        }
    }

    #[test]
    fn stream_preserves_cache_reporting_across_partial_usage_events() {
        let body = br#"data: {"message":{"usage":{"input_tokens":12,"output_tokens":0,"cache_read_input_tokens":0}}}
data: {"usage":{"output_tokens":8}}
data: [DONE]
"#;
        assert_eq!(
            parse_stream_usage(body),
            Some(TokenUsage {
                input: 12,
                output: 8,
                cached: Some(0)
            })
        );
        let body = br#"data: {"message":{"usage":{"input_tokens":12,"output_tokens":0,"cache_read_input_tokens":3,"cache_creation_input_tokens":4}}}
data: {"usage":{"output_tokens":8}}
"#;
        assert_eq!(
            parse_stream_usage(body),
            Some(TokenUsage {
                input: 19,
                output: 8,
                cached: Some(3)
            })
        );
    }

    #[test]
    fn identifies_native_protocols() {
        assert_eq!(
            request_protocol_model("/v1/chat/completions", &json!({"model":"chat"})).unwrap(),
            ("openai".to_owned(), "chat".to_owned())
        );
        assert_eq!(
            request_protocol_model("/v1/messages", &json!({"model":"claude"})).unwrap(),
            ("anthropic".to_owned(), "claude".to_owned())
        );
        assert_eq!(
            request_protocol_model("/v1beta/models/gemini-pro:generateContent", &json!({}))
                .unwrap(),
            ("gemini".to_owned(), "gemini-pro".to_owned())
        );
    }

    #[test]
    fn matches_exact_and_prefix_model_rules() {
        let rules = vec!["gpt-*".to_owned(), "claude-3-5-sonnet".to_owned()];
        assert!(model_rules::allows(&rules, "gpt-4.1"));
        assert!(model_rules::allows(&rules, "claude-3-5-sonnet"));
        assert!(!model_rules::allows(&rules, "gemini-2.5-pro"));
    }

    #[test]
    fn joins_v1_base_without_duplicate_prefix() {
        assert_eq!(
            join_base_url("https://example.com/v1", "/v1/chat/completions").unwrap(),
            "https://example.com/v1/chat/completions"
        );
        assert_eq!(
            join_base_url("https://example.com", "/v1/messages").unwrap(),
            "https://example.com/v1/messages"
        );
    }

    #[test]
    fn joins_other_versioned_bases_without_v1() {
        assert_eq!(
            join_base_url(
                "https://open.bigmodel.cn/api/paas/v4/",
                "/v1/chat/completions"
            )
            .unwrap(),
            "https://open.bigmodel.cn/api/paas/v4/chat/completions"
        );
        assert_eq!(
            join_base_url("https://ark.cn-beijing.volces.com/api/v3", "/v1/models").unwrap(),
            "https://ark.cn-beijing.volces.com/api/v3/models"
        );
        assert_eq!(
            join_base_url("https://api.deepseek.com/anthropic", "/v1/messages").unwrap(),
            "https://api.deepseek.com/anthropic/v1/messages"
        );
        assert_eq!(
            join_base_url("https://example.com/v1beta", "/v1beta/models").unwrap(),
            "https://example.com/v1beta/models"
        );
        assert_eq!(
            join_base_url("https://example.com/v", "/v1/models").unwrap(),
            "https://example.com/v/v1/models"
        );
        assert_eq!(
            join_base_url("https://example.com/v4", "/v1beta/models/g:generateContent").unwrap(),
            "https://example.com/v4/v1beta/models/g:generateContent"
        );
    }

    #[test]
    fn forwards_client_query_without_key() {
        let target = "https://example.com/v1beta/models/g:streamGenerateContent";
        assert_eq!(
            append_client_query(target, Some("alt=sse")),
            format!("{target}?alt=sse")
        );
        assert_eq!(
            append_client_query(target, Some("key=mr_x&alt=sse")),
            format!("{target}?alt=sse")
        );
        assert_eq!(append_client_query(target, Some("key=mr_x")), target);
        assert_eq!(append_client_query(target, None), target);
        assert_eq!(
            append_client_query("https://example.com/v1?tenant=a", Some("alt=sse&keyword=x")),
            "https://example.com/v1?tenant=a&alt=sse&keyword=x"
        );
    }

    #[test]
    fn upstream_errors_fail_over_and_count_by_kind() {
        let quota = r#"{"error":{"code":"insufficient_quota","message":"quota"}}"#;
        let credit = r#"{"type":"error","error":{"type":"invalid_request_error","message":"Your credit balance is too low"}}"#;
        let missing = r#"{"error":{"code":"model_not_found","message":"model not found"}}"#;
        let invalid = r#"{"error":{"message":"invalid temperature"}}"#;
        // (status, body, fails over, counted, refusal kind)
        let cases = [
            (429, invalid, true, false, None),
            (429, quota, true, true, Some("quota")),
            (402, invalid, true, true, Some("quota")),
            (400, credit, true, true, Some("quota")),
            (401, invalid, true, true, Some("credential")),
            (403, invalid, true, true, Some("credential")),
            (502, invalid, true, true, None),
            (408, invalid, true, false, None),
            (404, missing, true, false, None),
            (400, invalid, false, false, None),
        ];
        for (code, body, fails_over, counted, refusal) in cases {
            let status = StatusCode::from_u16(code).unwrap();
            let kind = CheckKind::classify(status, body.as_bytes());
            assert_eq!(kind.fails_over(status), fails_over, "{code}");
            let failure = kind.traffic_failure(status);
            assert_eq!(failure.is_some(), counted, "{code}");
            assert_eq!(failure.and_then(|(_, kind)| kind), refusal, "{code}");
        }
    }

    #[test]
    fn gemini_cache_miss_counts_as_reported() {
        assert_eq!(
            parse_usage(br#"{"usageMetadata":{"promptTokenCount":12,"candidatesTokenCount":8}}"#),
            Some(TokenUsage {
                input: 12,
                output: 8,
                cached: Some(0)
            })
        );
        let body = b"data: {\"usageMetadata\":{\"promptTokenCount\":7000,\"candidatesTokenCount\":0}}\n\ndata: {\"usageMetadata\":{\"promptTokenCount\":7000,\"candidatesTokenCount\":1,\"cachedContentTokenCount\":4960}}\n";
        assert_eq!(
            parse_stream_usage(body),
            Some(TokenUsage {
                input: 7000,
                output: 1,
                cached: Some(4960)
            })
        );
    }

    #[test]
    fn private_and_reserved_addresses_are_refused_in_every_spelling() {
        for host in [
            "127.0.0.1",
            "10.0.0.8",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "[::1]",
            "[::]",
            "[::ffff:127.0.0.1]",
            "[::ffff:a9fe:a9fe]",
            "[::127.0.0.1]",
            "[fd00::1]",
            "[fc00::1]",
            "[fe80::1]",
            "[fec0::1]",
            "[ff02::1]",
            "[64:ff9b::a00:1]",
            "[2002:7f00:1::]",
            "[2001:db8::1]",
            "localhost",
            "api.localhost",
            "printer.local",
            "metadata.google.internal",
            "LOCALHOST.",
        ] {
            assert!(blocked_host(host, false), "{host}");
            assert!(!blocked_host(host, true), "{host}");
        }
        for host in [
            "api.example.com",
            "1.1.1.1",
            "[2606:4700:4700::1111]",
            "[64:ff9b::101:101]",
        ] {
            assert!(!blocked_host(host, false), "{host}");
        }
    }

    #[tokio::test]
    async fn the_client_resolver_drops_private_addresses_at_connect_time() {
        use reqwest::dns::Resolve;
        let resolver = SafeResolver {
            trusted: HashSet::new(),
        };
        let name = "localhost".parse().unwrap();
        assert!(resolver.resolve(name).await.is_err());
        let trusted = SafeResolver {
            trusted: HashSet::from(["localhost".to_owned()]),
        };
        let name = "localhost".parse().unwrap();
        assert!(trusted.resolve(name).await.is_ok());
    }

    #[tokio::test]
    async fn refuses_upstream_bodies_over_the_limit() {
        let chunks = || {
            stream::iter(
                [b"0123".as_slice(), b"4567".as_slice()]
                    .map(|chunk| Ok::<_, ()>(axum::body::Bytes::from_static(chunk))),
            )
        };
        assert_eq!(
            collect_limited(chunks(), 8).await.as_deref(),
            Ok(b"01234567".as_slice())
        );
        assert_eq!(collect_limited(chunks(), 7).await, Err(BodyError::TooLarge));
        let broken = stream::iter([Err::<axum::body::Bytes, _>(())]);
        assert_eq!(collect_limited(broken, 8).await, Err(BodyError::Read));
    }

    #[test]
    fn stream_usage_is_the_same_however_the_stream_is_split() {
        let body = b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\ndata: {\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":5}}\n\ndata: [DONE]\n";
        let whole = parse_stream_usage(body);
        assert!(whole.is_some());
        for size in [1, 2, 3, 7, 64] {
            let mut usage = StreamUsage::default();
            for chunk in body.chunks(size) {
                usage.push(chunk);
            }
            assert_eq!(usage.finish(), whole, "chunk size {size}");
        }
        // An unterminated last line still counts once the stream ends.
        assert_eq!(parse_stream_usage(body.trim_ascii_end()), whole);
    }

    #[test]
    fn stream_usage_skips_overlong_lines_without_keeping_them() {
        let mut usage = StreamUsage::default();
        usage.push(b"data: {\"content\":\"");
        for _ in 0..(MAX_STREAM_LINE / 4096 + 1) {
            usage.push(&[b'x'; 4096]);
        }
        assert!(usage.line.is_empty());
        usage.push(b"\"}\ndata: {\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2}}\n");
        assert_eq!(
            usage.usage(),
            Some(TokenUsage {
                input: 3,
                output: 2,
                cached: None
            })
        );
        // The whole-stream copy for Gemini arrays is dropped once usage is known.
        usage.push(b"data: {}\n");
        assert!(usage.whole.is_none());
    }

    #[test]
    fn parses_gemini_json_array_streams_once_complete() {
        let body = b"[{\n  \"candidates\": []\n}\n,\r\n{\n  \"usageMetadata\": {\n    \"promptTokenCount\": 12,\n    \"candidatesTokenCount\": 8\n  }\n}\n]";
        assert!(parse_stream_usage(&body[..body.len() - 1]).is_none());
        assert_eq!(
            parse_stream_usage(body),
            Some(TokenUsage {
                input: 12,
                output: 8,
                cached: Some(0)
            })
        );
    }

    #[test]
    fn recognizes_anthropic_missing_model_only() {
        let missing = br#"{"type":"error","error":{"type":"not_found_error","message":"model: claude-missing"}}"#;
        assert!(model_not_supported(StatusCode::NOT_FOUND, missing));
        assert!(!model_not_supported(StatusCode::BAD_REQUEST, missing));
        let other = br#"{"type":"error","error":{"type":"not_found_error","message":"Not found"}}"#;
        assert!(!model_not_supported(StatusCode::NOT_FOUND, other));
    }

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, value.parse().unwrap());
        }
        map
    }

    #[test]
    fn reads_client_token_from_each_native_auth_header() {
        assert_eq!(
            client_token(&headers(&[("authorization", "Bearer mr_a")])),
            Some("mr_a")
        );
        assert_eq!(
            client_token(&headers(&[("x-api-key", "mr_b")])),
            Some("mr_b")
        );
        assert_eq!(
            client_token(&headers(&[("x-goog-api-key", "mr_c")])),
            Some("mr_c")
        );
        assert_eq!(
            client_token(&headers(&[
                ("authorization", "Bearer mr_a"),
                ("x-api-key", "mr_b")
            ])),
            Some("mr_a")
        );
    }

    #[test]
    fn rejects_missing_or_empty_client_token() {
        assert_eq!(client_token(&HeaderMap::new()), None);
        assert_eq!(
            client_token(&headers(&[("authorization", "Basic abc")])),
            None
        );
        assert_eq!(
            client_token(&headers(&[("authorization", "Bearer  ")])),
            None
        );
        assert_eq!(client_token(&headers(&[("x-api-key", " ")])), None);
    }
}
