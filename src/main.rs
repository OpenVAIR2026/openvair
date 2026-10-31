mod auth;
mod authenticity;
mod bili_tags;
mod channel_preview;
mod client_api;
mod crypto;
mod ctok;
mod edition;
mod email;
mod email_change;
mod error;
mod forwarded;
mod gateway;
mod key_groups;
mod key_holder;
mod key_quota;
mod key_share_links;
mod long_context;
mod metering;
mod model_rules;
mod models;
mod monitor;
mod password_change;
mod password_reset;
mod pricing;
mod providers;
mod public_url;
mod quality_monitors;
mod registration;
mod request_diagnostics;
mod routing;
mod secret_env;
mod service_checks;
mod sessions;
mod setup;
mod state;
mod system_admin;
mod token_estimate;
mod usage;
mod vair_key_channels;

#[cfg(test)]
#[path = "account_security_db_tests.rs"]
mod account_security_db_tests;
#[cfg(test)]
#[path = "channel_provider_db_tests.rs"]
mod channel_provider_db_tests;

use auth::{LoginInput, RegisterInput};
use axum::http::{HeaderName, HeaderValue, header::CACHE_CONTROL};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{any, delete, get, patch, post, put},
};
use chrono::{DateTime, Utc};
use crypto::Crypto;
use error::{AppError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, PgPool, Row, postgres::PgPoolOptions};
use state::AppState;
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    sync::Arc,
    time::Duration as StdDuration,
};
use tokio::sync::Mutex;
use tower_http::{
    cors::CorsLayer,
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeaderLayer,
    trace::TraceLayer,
};
use url::Url;
use uuid::Uuid;

#[derive(Clone, Serialize, FromRow)]
struct AccountView {
    id: Uuid,
    name: String,
    api_key_prefix: String,
    alert_emails: Value,
}

/// Which monitoring alerts are emailed to the account.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AlertEmails {
    authenticity: bool,
    service: bool,
    metering: bool,
    // Added later; clients that predate it keep the default.
    #[serde(default = "enabled")]
    availability: bool,
}

fn enabled() -> bool {
    true
}

#[derive(Clone, Serialize, Deserialize)]
struct AccountLimits {
    channel_limit: i64,
    virtual_key_limit: i64,
}

impl Default for AccountLimits {
    fn default() -> Self {
        Self {
            channel_limit: 20,
            virtual_key_limit: 50,
        }
    }
}

/// Model authenticity checks and the baselines they compare against. The
/// hosted service keeps them; OpenVAIR leaves them out for now.
fn authenticity_api() -> Router<AppState> {
    Router::new()
        .route(
            "/quality-monitors/{id}/deep-check",
            get(authenticity::deep::options).post(authenticity::deep::start),
        )
        .route(
            "/authenticity-options",
            get(authenticity::deep::new_options),
        )
        .route(
            "/authenticity-checks",
            get(authenticity::deep::list_account),
        )
        .route(
            "/quality-monitors/{id}/deep-checks",
            get(authenticity::deep::list),
        )
        .route(
            "/quality-monitors/{id}/deep-checks/{job}",
            get(authenticity::deep::get),
        )
        .route(
            "/platform/model-baselines",
            get(authenticity::baseline::list),
        )
        .route(
            "/platform/model-baselines/plans",
            post(authenticity::baseline::create_plan),
        )
        .route(
            "/platform/model-baselines/plans/{id}/download",
            get(authenticity::baseline::download_plan),
        )
        .route(
            "/platform/model-baselines/imports",
            get(authenticity::baseline::list_imports)
                .post(authenticity::baseline::import)
                .layer(DefaultBodyLimit::max(authenticity::bundle::MAX_BYTES)),
        )
        .route(
            "/platform/model-baselines/imports/{id}",
            get(authenticity::baseline::get_import),
        )
        .route(
            "/platform/model-baselines/imports/{id}/merge",
            post(authenticity::baseline::merge),
        )
        .route(
            "/platform/model-baselines/imports/{id}/new-version",
            post(authenticity::baseline::new_version),
        )
        .route(
            "/platform/model-baselines/imports/{id}/reject",
            post(authenticity::baseline::reject),
        )
        .route(
            "/platform/model-baselines/batches/{id}/exclude",
            post(authenticity::baseline::exclude),
        )
        .route(
            "/platform/model-baselines/versions/{id}/activate",
            post(authenticity::baseline::activate),
        )
        .route(
            "/platform/model-baselines/openrouter",
            get(authenticity::baseline::openrouter_lookup),
        )
        .route(
            "/platform/model-baselines/models/{model}",
            put(authenticity::baseline::put_target),
        )
        .route(
            "/platform/model-baselines/models/{model}/versions",
            get(authenticity::baseline::list_versions),
        )
        .route(
            "/platform/probe-templates",
            get(authenticity::baseline::list_templates)
                .post(authenticity::baseline::create_template),
        )
        .route(
            "/platform/probe-templates/{id}/retire",
            post(authenticity::baseline::retire_template),
        )
        .route(
            "/platform/probe-templates/defaults",
            post(authenticity::baseline::import_defaults),
        )
        .route(
            "/platform/probe-templates/publish",
            post(authenticity::baseline::publish_pool),
        )
}

/// Routes only the hosted service serves: open sign-up, public checks,
/// account administration, handing keys between accounts and signing the
/// desktop client in with an account. OpenVAIR Desktop only uses keys.
fn cloud_api() -> Router<AppState> {
    Router::new()
        .route("/client/device/authorize", post(client_api::authorize))
        .route("/client/device/token", post(client_api::token))
        .route("/client/device/{code}", get(client_api::show))
        .route("/client/device/{code}/approve", post(client_api::approve))
        .route("/client/device/{code}/deny", post(client_api::deny))
        .route("/client/me", get(client_api::me))
        .route("/client/keys", get(client_api::keys))
        .route("/client/keys/{id}/reveal", post(client_api::reveal))
        .route("/client/session", delete(client_api::sign_out))
        .route("/me/client-tokens", get(client_api::list_tokens))
        .route("/me/client-tokens/{id}", delete(client_api::revoke_token))
        .route("/auth/register", post(register))
        .route("/auth/register/code", post(registration::request_code))
        .route("/public/verify/models", get(authenticity::public::models))
        .route(
            "/public/verify/email-code",
            post(authenticity::public::request_code),
        )
        .route("/public/verify", post(authenticity::public::start))
        .route(
            "/public/verify/{token}",
            get(authenticity::public::get)
                .patch(authenticity::public::update)
                .delete(authenticity::public::delete),
        )
        .route("/auth/registration", get(system_admin::registration_status))
        .route(
            "/platform/account-limits",
            get(get_account_limits).patch(update_account_limits),
        )
        .route(
            "/platform/registration",
            get(system_admin::get_registration).patch(system_admin::update_registration),
        )
        .route(
            "/platform/public-verify",
            get(authenticity::public::get_capacity).patch(authenticity::public::update_capacity),
        )
        .route("/platform/accounts", get(system_admin::list_accounts))
        .route(
            "/platform/accounts/{id}/status",
            patch(system_admin::set_account_status),
        )
        .route(
            "/channels/vair-key",
            post(vair_key_channels::create_channel),
        )
        .route(
            "/channels/vair-key/resolve",
            post(vair_key_channels::resolve),
        )
        .route(
            "/channels/{id}/vair-key",
            post(vair_key_channels::replace_key),
        )
        .route(
            "/keys/{id}/share",
            post(key_share_links::create).delete(key_share_links::cancel),
        )
        .route("/key-shares/resolve", post(key_share_links::resolve))
        .route("/key-shares/accept", post(key_share_links::accept))
}

async fn load_account_limits(db: &PgPool) -> AccountLimits {
    // A self-hosted instance has one account, so nothing to share fairly.
    if !edition::CLOUD {
        return AccountLimits {
            channel_limit: i64::MAX,
            virtual_key_limit: i64::MAX,
        };
    }
    let row: Option<String> = sqlx::query_scalar(
        "SELECT value_ciphertext FROM system_settings WHERE key='account_limits'",
    )
    .fetch_optional(db)
    .await
    .ok()
    .flatten();
    row.and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

/// Checks the account's channel limit inside the transaction that inserts the
/// channel. The account row lock serializes concurrent creates, so two of them
/// cannot both see room for the last slot. NO KEY UPDATE leaves foreign-key
/// inserts that reference the account unblocked.
async fn reserve_channel_slot(
    conn: &mut sqlx::PgConnection,
    account_id: Uuid,
    channel_limit: i64,
) -> Result<()> {
    sqlx::query("SELECT 1 FROM accounts WHERE id=$1 FOR NO KEY UPDATE")
        .bind(account_id)
        .execute(&mut *conn)
        .await?;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM channels WHERE account_id=$1")
        .bind(account_id)
        .fetch_one(&mut *conn)
        .await?;
    if count >= channel_limit {
        return Err(AppError::Conflict("account channel limit reached".into()));
    }
    Ok(())
}

#[derive(Clone, Serialize, FromRow)]
struct ChannelView {
    id: Uuid,
    name: String,
    protocol: String,
    base_url: String,
    enabled: bool,
    priority: i32,
    /// Requests in flight through the channel at once; `None` means no cap.
    concurrency_limit: Option<i32>,
    health_status: String,
    health_error: Option<String>,
    health_error_kind: Option<String>,
    health_note: Option<String>,
    health_checked_at: Option<DateTime<Utc>>,
    test_model: Option<String>,
    models: Value,
    model_mapping: Value,
    model_mode: String,
    model_count: i32,
    discovered_models: Value,
    models_synced_at: Option<DateTime<Utc>>,
    #[sqlx(skip)]
    known_models: Vec<String>,
    created_at: DateTime<Utc>,
    kind: String,
    provider: Option<String>,
    #[serde(skip)]
    source_key_id: Option<Uuid>,
    // Owner, label, limits and status of the key behind a VAIR key channel.
    source: Option<Value>,
}

#[derive(Clone, Serialize, FromRow)]
struct MonitoringChannelView {
    id: Uuid,
    name: String,
    protocol: String,
    kind: String,
    enabled: bool,
    priority: i32,
    models: Value,
    test_model: Option<String>,
    availability_status: String,
    /// What the last check saw when it did not conclude, shown beside the verdict.
    availability_observation: Option<String>,
    availability_error: Option<String>,
    availability_error_kind: Option<String>,
    /// What the last check worked around, such as a retired test model.
    availability_note: Option<String>,
    availability_checked_at: Option<DateTime<Utc>>,
    availability_enabled: bool,
    availability_interval_seconds: i32,
    availability_failures: i32,
    availability_failure_threshold: i32,
    availability_route_policy: String,
    first_token_timeout_seconds: i32,
}

#[derive(Serialize)]
struct MonitoringSummaryView {
    total_channels: usize,
    available_channels: usize,
    channels: Vec<MonitoringChannelView>,
}

#[derive(Clone, Serialize, FromRow)]
struct MonitorRunView {
    id: Uuid,
    monitor_id: Option<Uuid>,
    channel_id: Uuid,
    kind: String,
    model: Option<String>,
    status: String,
    score: Option<f64>,
    latency_ms: Option<i32>,
    ttfb_ms: Option<i32>,
    status_code: Option<i32>,
    error_code: Option<String>,
    details: Value,
    started_at: DateTime<Utc>,
    finished_at: DateTime<Utc>,
}

#[derive(Clone, Serialize, FromRow)]
struct RouteView {
    id: Uuid,
    channel_id: Uuid,
    channel_name: String,
    public_model: String,
    upstream_model: String,
    protocol: String,
    priority: i32,
    enabled: bool,
}

#[derive(Clone, Serialize, FromRow)]
struct VirtualKeyView {
    id: Uuid,
    name: String,
    holder: Option<String>,
    key_prefix: String,
    can_copy: bool,
    allowed_models: Value,
    model_mode: String,
    #[sqlx(skip)]
    known_models: Vec<String>,
    /// `known_models` by protocol, for the desktop client.
    #[sqlx(skip)]
    #[serde(skip)]
    models: BTreeMap<String, BTreeSet<String>>,
    requests_per_minute: Option<i32>,
    token_limit_daily: Option<i64>,
    cost_limit_daily_usd: Option<f64>,
    cost_limit_total_usd: Option<f64>,
    concurrency_limit: i32,
    enabled: bool,
    created_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
    /// Usage counters follow the gateway's limit windows (UTC day, lifetime).
    used_tokens_today: i64,
    used_cost_today_usd: f64,
    used_cost_total_usd: f64,
    usage_reset_daily_at: Option<DateTime<Utc>>,
    usage_reset_total_at: Option<DateTime<Utc>>,
    channel_mode: String,
    channel_ids: Vec<Uuid>,
    /// While set, the channel and model scope is copied from this group.
    group_id: Option<Uuid>,
    // Accounts currently using this key as a channel.
    channel_count: i64,
    /// Of those, the ones that accepted a share link, as `{name, email}`;
    /// accounts that pasted the key stay anonymous.
    shared_accounts: Value,
    /// Expiry of the key's open share link, if any.
    share_expires_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Serialize, FromRow)]
struct ModelCatalogView {
    id: Uuid,
    pattern: String,
    label: String,
    enabled: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    provider: String,
    protocols: Vec<String>,
    input_per_1m: Option<f64>,
    output_per_1m: Option<f64>,
    cached_read_per_1m: Option<f64>,
    price_source: Option<String>,
}

#[derive(Clone, Serialize, FromRow)]
struct AuditView {
    id: Uuid,
    action: String,
    resource_type: Option<String>,
    resource_id: Option<Uuid>,
    metadata: Value,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct CreateChannelInput {
    name: String,
    protocol: String,
    base_url: String,
    priority: Option<i32>,
    models: Option<Vec<String>>,
    model_mode: Option<String>,
    test_model: Option<String>,
    secret: String,
    check_receipt: Option<String>,
    provider: Option<String>,
    concurrency_limit: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct PreviewChannelModelsInput {
    channel_id: Option<Uuid>,
    protocol: String,
    base_url: String,
    secret: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PreviewChannelCheckInput {
    #[serde(flatten)]
    connection: PreviewChannelModelsInput,
    model: String,
}

#[derive(Debug, Deserialize)]
struct UpdateChannelModelsInput {
    models: Vec<String>,
    model_mode: Option<String>,
    #[serde(default, deserialize_with = "deserialize_nullable")]
    test_model: Option<Option<String>>,
}

#[derive(Deserialize)]
struct UpdateChannelInput {
    name: Option<String>,
    base_url: Option<String>,
    priority: Option<i32>,
    models: Option<Vec<String>>,
    model_mapping: Option<Value>,
    model_mode: Option<String>,
    #[serde(default, deserialize_with = "deserialize_nullable")]
    test_model: Option<Option<String>>,
    enabled: Option<bool>,
    secret: Option<String>,
    check_receipt: Option<String>,
    #[serde(default, deserialize_with = "deserialize_nullable")]
    concurrency_limit: Option<Option<i32>>,
}

fn deserialize_nullable<'de, D, T>(
    deserializer: D,
) -> std::result::Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Deserialize)]
struct CreateRouteInput {
    channel_id: Uuid,
    public_model: String,
    upstream_model: String,
    protocol: String,
    priority: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct CreateVirtualKeyInput {
    name: String,
    /// Who the key was issued to.
    holder: Option<String>,
    allowed_models: Vec<String>,
    model_mode: Option<String>,
    requests_per_minute: Option<i32>,
    token_limit_daily: Option<i64>,
    cost_limit_daily_usd: Option<f64>,
    cost_limit_total_usd: Option<f64>,
    concurrency_limit: Option<i32>,
    channel_mode: Option<String>,
    #[serde(default)]
    channel_ids: Vec<Uuid>,
    /// A group replaces the channel and model fields with its own scope.
    group_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
struct UpdateVirtualKeyInput {
    name: String,
    /// Omitted keeps the current holder; null or empty clears it.
    #[serde(default, deserialize_with = "deserialize_nullable")]
    holder: Option<Option<String>>,
    allowed_models: Vec<String>,
    model_mode: Option<String>,
    requests_per_minute: Option<i32>,
    token_limit_daily: Option<i64>,
    cost_limit_daily_usd: Option<f64>,
    /// Omitted keeps the current cap so older clients cannot clear it.
    #[serde(default, deserialize_with = "deserialize_nullable")]
    cost_limit_total_usd: Option<Option<f64>>,
    concurrency_limit: i32,
    enabled: Option<bool>,
    /// Omitted keeps the key's channel scope.
    channel_mode: Option<String>,
    #[serde(default)]
    channel_ids: Vec<Uuid>,
    /// Omitted keeps the key's group; null leaves it with its current scope.
    #[serde(default, deserialize_with = "deserialize_nullable")]
    group_id: Option<Option<Uuid>>,
}

#[derive(Debug, Deserialize)]
struct ModelCatalogInput {
    pattern: String,
    label: String,
    enabled: Option<bool>,
    provider: Option<String>,
    protocols: Option<Vec<String>>,
    price: Option<pricing::ModelPrice>,
    #[serde(default)]
    clear_price: bool,
    #[serde(default)]
    only_unpriced: bool,
}

#[derive(Debug, Deserialize)]
struct UpdateAccountSettingsInput {
    api_key_prefix: Option<String>,
    alert_emails: Option<AlertEmails>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateAccountInput {
    account_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateEmailSettingsInput {
    host: String,
    port: u16,
    security: String,
    username: Option<String>,
    password: Option<String>,
    from: String,
}

#[derive(Debug, Serialize)]
struct EmailSettingsView {
    configured: bool,
    host: String,
    port: u16,
    security: String,
    username: Option<String>,
    from: String,
    password_configured: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateMonitorInput {
    availability_enabled: Option<bool>,
    availability_interval_seconds: Option<i32>,
    availability_failure_threshold: Option<i32>,
    availability_route_policy: Option<String>,
    first_token_timeout_seconds: Option<i32>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum MonitorKind {
    Availability,
    Quality,
    /// Service check records: rounds and long-context checks.
    Service,
}

impl MonitorKind {
    fn kinds(self) -> Vec<&'static str> {
        match self {
            Self::Availability => vec!["availability"],
            Self::Quality => vec!["quality"],
            Self::Service => vec!["quality", "long_context"],
        }
    }
}

#[derive(Debug, Deserialize)]
struct MonitorRunsQuery {
    monitor_id: Option<Uuid>,
    channel_id: Option<Uuid>,
    kind: Option<MonitorKind>,
    limit: Option<i64>,
    page: Option<i64>,
    /// Return the latest N runs for every channel and kind instead of a global
    /// list, so frequent availability checks cannot crowd out quality runs.
    per_channel: Option<i64>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let database_url = secret_env::required("DATABASE_URL").map_err(AppError::Internal)?;
    let redis_url = std::env::var("REDIS_URL")
        .map_err(|_| AppError::Internal("REDIS_URL is required".into()))?;
    let db = PgPoolOptions::new()
        .max_connections(20)
        .connect(&database_url)
        .await
        .map_err(|e| AppError::Internal(format!("database connection: {e}")))?;
    sqlx::migrate!("./migrations")
        .run(&db)
        .await
        .map_err(|e| AppError::Internal(format!("database migration: {e}")))?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("admin") {
        return setup::run_admin_command(&db, &args[1..]).await;
    }
    let redis = redis::Client::open(redis_url)
        .map_err(|e| AppError::Internal(format!("redis client: {e}")))?;
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .dns_resolver(Arc::new(gateway::SafeResolver::from_env()))
        .timeout(gateway::UPSTREAM_TIMEOUT)
        .build()
        .map_err(|e| AppError::Internal(format!("http client: {e}")))?;
    let cookie_secure = std::env::var("COOKIE_SECURE").map_or(true, |value| value != "false");
    if !cookie_secure {
        eprintln!(
            "warning: COOKIE_SECURE=false sends session cookies over plain HTTP; keep it on behind HTTPS"
        );
    }
    forwarded::configure(std::env::var("TRUSTED_PROXIES").ok().as_deref())
        .map_err(AppError::Internal)?;
    let super_admin_emails = std::env::var("SUPER_ADMIN_EMAILS")
        .unwrap_or_default()
        .split(',')
        .map(|email| email.trim().to_ascii_lowercase())
        .filter(|email| !email.is_empty())
        .collect::<HashSet<_>>();
    gateway::allow_private_upstreams(
        std::env::var("ALLOW_PRIVATE_UPSTREAMS").map_or(true, |value| value != "false"),
    );
    system_admin::mark_started();
    let state = AppState {
        db,
        redis,
        http,
        crypto: Arc::new(
            Crypto::from_env().map_err(|message| AppError::Internal(message.to_owned()))?,
        ),
        semaphores: Arc::new(Mutex::new(std::collections::HashMap::new())),
        cookie_secure,
        super_admin_emails: Arc::new(super_admin_emails),
        redis_connection: Default::default(),
        routing_settings: Default::default(),
        prices: Default::default(),
    };
    // Without prices every request would be recorded as unpriced for good.
    pricing::set(
        &state,
        pricing::load(&state.db)
            .await
            .map_err(|e| AppError::Internal(format!("model prices: {e}")))?,
    );

    let cleanup_db = state.db.clone();
    tokio::spawn(async move {
        loop {
            let _ = sqlx::query(
                "DELETE FROM request_logs WHERE expires_at IS NOT NULL AND expires_at <= now()",
            )
            .execute(&cleanup_db)
            .await;
            let _ = sqlx::query("DELETE FROM sessions WHERE expires_at <= now()")
                .execute(&cleanup_db)
                .await;
            let _ = sqlx::query("DELETE FROM email_changes WHERE expires_at <= now()")
                .execute(&cleanup_db)
                .await;
            let _ = sqlx::query("DELETE FROM password_resets WHERE expires_at <= now()")
                .execute(&cleanup_db)
                .await;
            let _ = sqlx::query("DELETE FROM registration_verifications WHERE expires_at <= now()")
                .execute(&cleanup_db)
                .await;
            let _ = sqlx::query(
                "DELETE FROM client_device_authorizations WHERE expires_at <= now()-interval '1 day'",
            )
            .execute(&cleanup_db)
            .await;
            let _ = sqlx::query(
                "DELETE FROM client_tokens WHERE expires_at <= now() OR revoked_at <= now()-interval '30 days'",
            )
            .execute(&cleanup_db)
            .await;
            tokio::time::sleep(StdDuration::from_secs(3600)).await;
        }
    });
    let health_state = state.clone();
    tokio::spawn(async move {
        tokio::time::sleep(StdDuration::from_secs(30)).await;
        loop {
            system_admin::beat(&system_admin::AVAILABILITY_WORKER_BEAT);
            if let Ok(rows) = sqlx::query(
                "SELECT c.id,c.account_id
                 FROM channels c
                 JOIN accounts a ON a.id=c.account_id AND a.disabled_at IS NULL
                 LEFT JOIN channel_monitor_configs mc ON mc.channel_id=c.id
                 WHERE c.enabled=true
                   AND COALESCE(mc.availability_enabled,c.kind='upstream')
                   AND (mc.next_availability_at IS NULL OR mc.next_availability_at <= now())
                 ORDER BY mc.next_availability_at NULLS FIRST
                 LIMIT 20",
            )
            .fetch_all(&health_state.db)
            .await
            {
                for row in rows {
                    let channel_id: std::result::Result<Uuid, _> = sqlx::Row::try_get(&row, "id");
                    let account_id: std::result::Result<Uuid, _> =
                        sqlx::Row::try_get(&row, "account_id");
                    if let (Ok(channel_id), Ok(account_id)) = (channel_id, account_id) {
                        let _ = gateway::probe_channel(&health_state, account_id, channel_id).await;
                        // A recorded check already scheduled the next one, sooner after a failure.
                        let _ = sqlx::query(
                            "UPDATE channel_monitor_configs
                             SET next_availability_at=now() + make_interval(secs => availability_interval_seconds), updated_at=now()
                             WHERE channel_id=$1 AND (next_availability_at IS NULL OR next_availability_at<=now())",
                        )
                        .bind(channel_id)
                        .execute(&health_state.db)
                        .await;
                    }
                }
            }
            tokio::time::sleep(StdDuration::from_secs(30)).await;
        }
    });
    if !edition::CLOUD {
        setup::prepare(&state.db).await?;
    }
    pricing::start_refresh(state.clone());
    routing::start_refresh(state.clone());
    monitor::start_worker(state.clone());
    if edition::CLOUD {
        authenticity::public::start_worker(state.clone());
    }

    let api = Router::new()
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/password-reset", post(password_reset::request))
        .route(
            "/auth/password-reset/confirm",
            post(password_reset::confirm),
        )
        .route("/server", get(public_url::server_info))
        .route("/key/summary", get(key_holder::summary))
        .route("/key/usage", get(key_holder::usage))
        .route("/key/events", get(key_holder::events))
        .route("/key/events/{id}", get(key_holder::detail))
        .route("/key/quota-events", get(key_holder::quota_history))
        .route("/me/sessions", get(sessions::list))
        .route("/me/sessions/revoke-others", post(sessions::revoke_others))
        .route("/me/sessions/{id}", delete(sessions::revoke))
        .route("/me/email-verification", post(email_change::request))
        .route(
            "/me/email-verification/confirm",
            post(email_change::confirm),
        )
        .route("/me/password", post(password_change::change))
        .route("/me", get(me).patch(update_account))
        .route("/account", get(account).patch(update_account_settings))
        .route(
            "/model-catalog",
            get(list_model_catalog).post(create_model_catalog),
        )
        .route(
            "/model-catalog/{id}",
            patch(update_model_catalog).delete(delete_model_catalog),
        )
        .route(
            "/platform/email-settings",
            get(get_email_settings).patch(update_email_settings),
        )
        .route("/platform/email-settings/test", post(test_email_settings))
        .route(
            "/platform/public-url",
            get(public_url::get_setting).patch(public_url::update_setting),
        )
        .route(
            "/platform/routing-settings",
            get(routing::get_settings).patch(routing::update_settings),
        )
        .route(
            "/platform/model-pricing",
            get(get_model_pricing)
                .put(put_model_pricing)
                .post(add_model_pricing),
        )
        .route("/platform/health", get(system_admin::system_health))
        .route("/platform/unknown-cost", get(system_admin::unknown_cost))
        .route("/platform/model-pricing/unpriced", get(get_unpriced_models))
        .route("/platform/model-pricing/sync", post(sync_model_pricing))
        .route(
            "/platform/model-pricing/sync-preview",
            get(preview_model_pricing_sync),
        )
        .route(
            "/platform/model-pricing/sync-one",
            post(sync_one_model_pricing),
        )
        .route(
            "/platform/model-pricing/catalog",
            get(get_model_pricing_catalog),
        )
        .route("/available-models", get(models::available))
        .route("/channels", get(list_channels).post(create_channel))
        .route("/providers", get(providers::list))
        .route(
            "/channels/{id}",
            delete(delete_channel).patch(update_channel),
        )
        .route("/channels/concurrency", get(channel_concurrency))
        .route("/channels/preview-models", post(preview_channel_models))
        .route("/channels/preview-check", post(preview_channel_check))
        .route("/channels/{id}/check", post(check_channel))
        .route(
            "/channels/{id}/quota-events",
            get(key_quota::channel_history),
        )
        .route(
            "/channels/{id}/monitor",
            get(get_channel_monitor).patch(update_channel_monitor),
        )
        .route(
            "/quality-monitors",
            get(quality_monitors::list).post(quality_monitors::create),
        )
        .route(
            "/quality-monitors/{id}",
            patch(quality_monitors::update).delete(quality_monitors::delete),
        )
        .route("/quality-monitors/{id}/run", post(quality_monitors::run))
        .route("/passive-metering", get(metering::list))
        .route(
            "/quality-monitors/{id}/long-context",
            get(long_context::options).post(long_context::start),
        )
        .route("/channels/{id}/sync-models", post(sync_channel_models))
        .route("/channels/{id}/models", patch(update_channel_models))
        .route("/routes", get(list_routes).post(create_route))
        .route("/routes/{id}", delete(delete_route))
        .route("/keys", get(list_keys).post(create_key))
        .route(
            "/keys/groups",
            get(key_groups::list).post(key_groups::create),
        )
        .route(
            "/keys/groups/{id}",
            patch(key_groups::update).delete(key_groups::delete),
        )
        .route("/keys/{id}", delete(revoke_key).patch(update_key))
        .route("/keys/{id}/reveal", post(reveal_key))
        .route("/keys/{id}/enabled", patch(set_key_enabled))
        .route("/keys/{id}/usage-reset", post(reset_key_usage))
        .route("/keys/{id}/quota-increases", post(key_quota::increase))
        .route("/keys/{id}/quota-events", get(key_quota::owner_history))
        .route("/usage", get(usage::recent))
        .route("/usage/summary", get(usage::summary))
        .route("/usage/events", get(usage::events))
        .route("/usage/events/{id}", get(usage::detail))
        .route("/monitoring/summary", get(monitoring_summary))
        .route("/monitoring/runs", get(monitoring_runs))
        .route("/audit", get(audit_events));
    let api = if edition::CLOUD {
        api.merge(cloud_api()).merge(authenticity_api())
    } else {
        api.merge(setup::routes())
    }
    .with_state(state.clone());

    // Desktop installers live in a read-only directory mounted from the host,
    // so a new client release does not need a new server image.
    let downloads_dir = std::env::var("DOWNLOADS_DIR").unwrap_or_else(|_| "downloads".into());
    let mut app = Router::new().nest("/api", api);
    if edition::CLOUD {
        app = app.nest_service("/downloads", ServeDir::new(downloads_dir));
    }
    let app = app
        .route("/healthz", get(healthz))
        .route("/v1/{*path}", any(gateway::handle))
        .route("/v1beta/{*path}", any(gateway::handle))
        // Hashed assets never change, and a missing one must stay a 404 rather
        // than index.html so stale tabs see a real load error.
        .nest_service(
            "/assets",
            Router::new()
                .fallback_service(ServeDir::new("web/dist/assets"))
                .layer(SetResponseHeaderLayer::if_not_present(
                    CACHE_CONTROL,
                    HeaderValue::from_static("public, max-age=31536000, immutable"),
                )),
        )
        .fallback_service(
            Router::new()
                .fallback_service(
                    ServeDir::new("web/dist")
                        .append_index_html_on_directories(true)
                        .fallback(ServeFile::new("web/dist/index.html")),
                )
                .layer(SetResponseHeaderLayer::if_not_present(
                    CACHE_CONTROL,
                    HeaderValue::from_static("no-cache"),
                )),
        )
        .layer(SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static("content-security-policy"),
            HeaderValue::from_static(CONTENT_SECURITY_POLICY),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static("strict-transport-security"),
            HeaderValue::from_static("max-age=31536000"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static("x-frame-options"),
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static("referrer-policy"),
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        ))
        .layer(axum::middleware::from_fn(forwarded::middleware))
        .layer(CorsLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = std::env::var("APP_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| AppError::Internal(format!("listen: {e}")))?;
    println!("OpenVAIR listening on {addr}");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .map_err(|e| AppError::Internal(format!("server: {e}")))
}

/// The console is a same-origin SPA; Ant Design injects inline styles, so
/// only styles get `'unsafe-inline'`.
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'; \
    style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; \
    connect-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self'; \
    frame-ancestors 'none'";

async fn healthz(State(state): State<AppState>) -> Result<Json<Value>> {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.db)
        .await
        .map_err(|_| AppError::Internal("database health check failed".into()))?;
    Ok(Json(json!({"status":"ok"})))
}

async fn register(
    State(state): State<AppState>,
    request_headers: HeaderMap,
    Json(input): Json<RegisterInput>,
) -> Result<Response> {
    let email = auth::normalize_email(&input.email)?;
    system_admin::ensure_registration_allowed(&state, &email).await?;
    auth::validate_password(&input.password)?;
    let account_name = input.account_name.trim();
    if account_name.is_empty() || account_name.len() > 120 {
        return Err(AppError::BadRequest(
            "帐号名称不能为空，且不能超过 120 字节".into(),
        ));
    }
    let password_hash = auth::hash_password(&input.password)?;
    let settings = system_admin::load_registration(&state.db).await;
    let verify = system_admin::email_verification_required(&state, &settings).await;
    let mut tx = state.db.begin().await?;
    if verify {
        let check =
            registration::consume_code(&mut tx, &email, input.request_id, input.code).await?;
        if let registration::Check::Rejected(error) = check {
            // Keep the counted attempt even though registration stops here.
            tx.commit().await?;
            return Err(error);
        }
    }
    let user = sqlx::query("INSERT INTO users(email,password_hash) VALUES ($1,$2) RETURNING id")
        .bind(&email)
        .bind(password_hash)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            if e.to_string().contains("duplicate") {
                AppError::Conflict("email is already registered".into())
            } else {
                e.into()
            }
        })?;
    let user_id: Uuid = sqlx::Row::try_get(&user, "id")
        .map_err(|_| AppError::Internal("invalid user row".into()))?;
    let account = sqlx::query("INSERT INTO accounts(name) VALUES ($1) RETURNING id")
        .bind(account_name)
        .fetch_one(&mut *tx)
        .await?;
    let account_id: Uuid = sqlx::Row::try_get(&account, "id")
        .map_err(|_| AppError::Internal("invalid account row".into()))?;
    sqlx::query("INSERT INTO memberships(account_id,user_id) VALUES ($1,$2)")
        .bind(account_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    let session_token = auth::insert_session(&mut *tx, user_id, &request_headers).await?;
    tx.commit().await?;
    let mut headers = HeaderMap::new();
    auth::set_cookie(
        &mut headers,
        &session_token,
        auth::cookie_secure(state.cookie_secure, &request_headers),
    );
    Ok((
        StatusCode::CREATED,
        headers,
        Json(json!({"user_id":user_id,"account_id":account_id,"email":email})),
    )
        .into_response())
}

async fn login(
    State(state): State<AppState>,
    request_headers: HeaderMap,
    Json(input): Json<LoginInput>,
) -> Result<Response> {
    let email = auth::normalize_email(&input.email)?;
    let ip = auth::client_ip(&request_headers);
    auth::ensure_login_allowed(&state, &email, ip.as_deref()).await?;
    let row: Option<(Uuid, String, bool)> = sqlx::query_as(
        "SELECT u.id,u.password_hash,a.disabled_at IS NOT NULL FROM users u
         JOIN memberships m ON m.user_id=u.id JOIN accounts a ON a.id=m.account_id
         WHERE u.email=$1",
    )
    .bind(&email)
    .fetch_optional(&state.db)
    .await?;
    let (user_id, password_hash, disabled) = match row {
        Some((id, hash, disabled)) => (Some(id), Some(hash), disabled),
        None => (None, None, false),
    };
    let valid = auth::verify_password_blocking(input.password, password_hash).await?;
    let Some(user_id) = user_id.filter(|_| valid) else {
        auth::record_login_failure(&state, &email, ip.as_deref()).await;
        return Err(AppError::Unauthorized);
    };
    auth::clear_login_failures(&state, &email).await;
    // Only reveal the suspension once the password proves ownership.
    if disabled {
        return Err(AppError::Refused(
            "account_disabled",
            "帐号已被停用，请联系管理员".into(),
        ));
    }
    let token = auth::insert_session(&state.db, user_id, &request_headers).await?;
    let mut headers = HeaderMap::new();
    auth::set_cookie(
        &mut headers,
        &token,
        auth::cookie_secure(state.cookie_secure, &request_headers),
    );
    Ok((headers, Json(json!({"ok":true,"email":email}))).into_response())
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Result<Response> {
    if let Some(token) = auth::read_cookie(&headers, "gateway_session") {
        sqlx::query("DELETE FROM sessions WHERE token_hash=$1")
            .bind(auth::hash_token(&token))
            .execute(&state.db)
            .await?;
    }
    let mut response_headers = HeaderMap::new();
    auth::clear_cookie(
        &mut response_headers,
        auth::cookie_secure(state.cookie_secure, &headers),
    );
    Ok((response_headers, Json(json!({"ok":true}))).into_response())
}

async fn me(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    sessions::touch(&state.db, &headers).await;
    Ok(Json(json!(user)))
}

async fn update_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<UpdateAccountInput>,
) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    let account_name = input
        .account_name
        .map(|value| {
            let name = value.trim().to_owned();
            if name.is_empty() || name.len() > 120 {
                Err(AppError::BadRequest(
                    "帐号名称不能为空，且不能超过 120 字节".into(),
                ))
            } else {
                Ok(name)
            }
        })
        .transpose()?;
    let mut tx = state.db.begin().await?;
    if let Some(name) = account_name.as_deref() {
        sqlx::query("UPDATE accounts SET name=$1 WHERE id=$2")
            .bind(name)
            .bind(user.account_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    if account_name.is_some() {
        audit(
            &state.db,
            user.account_id,
            user.user_id,
            "account.name_updated",
            "account",
            user.account_id,
            json!({}),
        )
        .await;
    }
    Ok(Json(json!(auth::current_user(&state, &headers).await?)))
}

async fn account(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    let result = sqlx::query_as::<_, AccountView>(
        "SELECT id,name,api_key_prefix,alert_emails FROM accounts WHERE id=$1",
    )
    .bind(user.account_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    let limits = load_account_limits(&state.db).await;
    Ok(Json(json!({
        "id": result.id,
        "name": result.name,
        "api_key_prefix": result.api_key_prefix,
        "alert_emails": result.alert_emails,
        // Null when unlimited, as on a self-hosted instance.
        "channel_limit": edition::CLOUD.then_some(limits.channel_limit),
        "virtual_key_limit": edition::CLOUD.then_some(limits.virtual_key_limit),
    })))
}

async fn update_account_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<UpdateAccountSettingsInput>,
) -> Result<Json<AccountView>> {
    let user = auth::current_user(&state, &headers).await?;
    let current = get_account(&state.db, user.account_id).await?;
    let api_key_prefix = input
        .api_key_prefix
        .as_deref()
        .map(normalize_api_key_prefix)
        .transpose()?
        .unwrap_or(current.api_key_prefix);
    let alert_emails = input
        .alert_emails
        .map(|v| json!(v))
        .unwrap_or(current.alert_emails);
    sqlx::query("UPDATE accounts SET api_key_prefix=$1,alert_emails=$3 WHERE id=$2")
        .bind(&api_key_prefix)
        .bind(user.account_id)
        .bind(&alert_emails)
        .execute(&state.db)
        .await?;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "account.settings_updated",
        "account",
        user.account_id,
        json!({"api_key_prefix":api_key_prefix,"alert_emails":alert_emails}),
    )
    .await;
    Ok(Json(get_account(&state.db, user.account_id).await?))
}

fn normalize_model_pattern(value: &str) -> Result<String> {
    model_rules::normalize(value)
}

fn normalize_api_key_prefix(value: &str) -> Result<String> {
    let prefix = value.trim();
    if prefix.is_empty() || prefix.len() > 32 {
        return Err(AppError::BadRequest(
            "API Key 前缀必须为 1-32 个字符".into(),
        ));
    }
    let mut chars = prefix.chars();
    if !chars.next().is_some_and(|ch| ch.is_ascii_alphanumeric())
        || !chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        return Err(AppError::BadRequest(
            "API Key 前缀只能包含字母、数字、连字符和下划线，且必须以字母或数字开头".into(),
        ));
    }
    Ok(prefix.to_owned())
}

fn normalize_channel_models(values: Vec<String>) -> Result<Vec<String>> {
    let mut models = values
        .into_iter()
        .map(|model| model.trim().to_owned())
        .filter(|model| !model.is_empty())
        .collect::<Vec<_>>();
    if models.len() > 1000 {
        return Err(AppError::BadRequest(
            "channel model list is too large".into(),
        ));
    }
    for model in &models {
        normalize_model_pattern(model)?;
    }
    models.sort();
    models.dedup();
    Ok(models)
}

fn validate_model_catalog_label(value: &str) -> Result<&str> {
    let label = value.trim();
    if label.is_empty() || label.len() > 120 {
        return Err(AppError::BadRequest(
            "model label must be 1-120 characters".into(),
        ));
    }
    Ok(label)
}

fn normalize_allowed_models(values: Vec<String>) -> Result<Vec<String>> {
    let mut models = values
        .into_iter()
        .map(|model| model.trim().to_owned())
        .filter(|model| !model.is_empty())
        .collect::<Vec<_>>();
    if models.len() > 100 {
        return Err(AppError::BadRequest(
            "a key can allow at most 100 model rules".into(),
        ));
    }
    for model in &models {
        normalize_model_pattern(model)?;
    }
    models.sort();
    models.dedup();
    Ok(models)
}

fn email_settings_view(config: Option<&email::SmtpConfig>) -> EmailSettingsView {
    if let Some(config) = config {
        return EmailSettingsView {
            configured: true,
            host: config.host.clone(),
            port: config.port,
            security: config.security.clone(),
            username: config.username.clone(),
            from: config.from.clone(),
            password_configured: config.password.is_some(),
        };
    }
    EmailSettingsView {
        configured: false,
        host: String::new(),
        port: 587,
        security: "starttls".into(),
        username: None,
        from: String::new(),
        password_configured: false,
    }
}

fn validate_smtp_text(value: &str, field: &str, max_len: usize) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > max_len || value.chars().any(char::is_control) {
        return Err(AppError::BadRequest(format!("{field}格式无效")));
    }
    Ok(value.to_owned())
}

async fn get_email_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<EmailSettingsView>> {
    auth::require_super_admin(&state, &headers).await?;
    let config = email::load_smtp_config_optional(&state).await?;
    Ok(Json(email_settings_view(config.as_ref())))
}

async fn test_email_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let user = auth::require_super_admin(&state, &headers).await?;
    let smtp = email::load_smtp_config(&state).await?;
    email::send_test_email(smtp, user.email.clone()).await?;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "platform.email_settings_test_sent",
        "system_settings",
        user.user_id,
        json!({}),
    )
    .await;
    Ok(Json(json!({ "sent_to": user.email })))
}

async fn update_email_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<UpdateEmailSettingsInput>,
) -> Result<Json<EmailSettingsView>> {
    let user = auth::require_super_admin(&state, &headers).await?;
    let host = validate_smtp_text(&input.host, "SMTP 服务器", 255)?;
    if input.port == 0 {
        return Err(AppError::BadRequest("SMTP 端口无效".into()));
    }
    let security = input.security.trim().to_ascii_lowercase();
    if !matches!(security.as_str(), "starttls" | "tls") {
        return Err(AppError::BadRequest(
            "加密方式必须是 STARTTLS 或 TLS".into(),
        ));
    }
    let from = validate_smtp_text(&input.from, "发件地址", 320)?;
    let from = from
        .parse::<lettre::Address>()
        .map_err(|_| AppError::BadRequest("请输入有效的发件邮箱地址".into()))?
        .to_string();
    let username = input
        .username
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if username
        .as_deref()
        .is_some_and(|value| value.len() > 320 || value.chars().any(char::is_control))
    {
        return Err(AppError::BadRequest("SMTP 用户名格式无效".into()));
    }
    let existing = email::load_smtp_config_optional(&state).await?;
    let password_input = input
        .password
        .and_then(|value| (!value.trim().is_empty()).then_some(value));
    let password = match username.as_ref() {
        None => None,
        Some(username) => match password_input {
            Some(password) if password.len() <= 4096 => Some(password),
            Some(_) => return Err(AppError::BadRequest("SMTP 密码过长".into())),
            None if existing
                .as_ref()
                .and_then(|config| config.username.as_ref())
                .is_some_and(|old_username| old_username == username) =>
            {
                existing.as_ref().and_then(|config| config.password.clone())
            }
            None => {
                return Err(AppError::BadRequest(
                    "请输入 SMTP 密码；如无需认证请清空 SMTP 用户名".into(),
                ));
            }
        },
    };
    if username.is_some() && password.is_none() {
        return Err(AppError::BadRequest("请输入 SMTP 密码".into()));
    }
    let config = email::SmtpConfig {
        host,
        port: input.port,
        security,
        username,
        password,
        from,
    };
    let plaintext = serde_json::to_string(&config)
        .map_err(|_| AppError::Internal("邮箱配置序列化失败".into()))?;
    let ciphertext = state
        .crypto
        .encrypt(&plaintext)
        .map_err(AppError::Internal)?;
    sqlx::query(
        "INSERT INTO system_settings(key,value_ciphertext) VALUES ('smtp',$1)
         ON CONFLICT (key) DO UPDATE SET value_ciphertext=EXCLUDED.value_ciphertext,updated_at=now()",
    )
    .bind(ciphertext)
    .execute(&state.db)
    .await?;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "platform.email_settings_updated",
        "system_settings",
        user.user_id,
        json!({"host": &config.host, "port": config.port, "security": &config.security, "username": &config.username, "from": &config.from}),
    )
    .await;
    Ok(Json(email_settings_view(Some(&config))))
}

async fn get_account_limits(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AccountLimits>> {
    auth::require_super_admin(&state, &headers).await?;
    Ok(Json(load_account_limits(&state.db).await))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateAccountLimitsInput {
    channel_limit: Option<i64>,
    virtual_key_limit: Option<i64>,
}

async fn update_account_limits(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<UpdateAccountLimitsInput>,
) -> Result<Json<AccountLimits>> {
    let user = auth::require_super_admin(&state, &headers).await?;
    let current = load_account_limits(&state.db).await;
    let updated = AccountLimits {
        channel_limit: input.channel_limit.unwrap_or(current.channel_limit).max(1),
        virtual_key_limit: input
            .virtual_key_limit
            .unwrap_or(current.virtual_key_limit)
            .max(1),
    };
    let json = serde_json::to_string(&updated)
        .map_err(|_| AppError::Internal("limits serialization failed".into()))?;
    sqlx::query(
        "INSERT INTO system_settings(key,value_ciphertext) VALUES ('account_limits',$1)
         ON CONFLICT (key) DO UPDATE SET value_ciphertext=EXCLUDED.value_ciphertext,updated_at=now()",
    )
    .bind(&json)
    .execute(&state.db)
    .await?;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "platform.account_limits_updated",
        "system_settings",
        user.user_id,
        json!({"channel_limit":updated.channel_limit,"virtual_key_limit":updated.virtual_key_limit}),
    )
    .await;
    Ok(Json(updated))
}

async fn get_model_pricing(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<pricing::ModelPrice>>> {
    auth::require_super_admin(&state, &headers).await?;
    Ok(Json(pricing::load(&state.db).await?))
}

async fn put_model_pricing(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(prices): Json<Vec<pricing::ModelPrice>>,
) -> Result<Json<Vec<pricing::ModelPrice>>> {
    let user = auth::require_super_admin(&state, &headers).await?;
    pricing::save(&state.db, &prices).await?;
    pricing::refresh(&state).await;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "platform.model_pricing_replaced",
        "model_pricing",
        user.user_id,
        json!({"count":prices.len()}),
    )
    .await;
    Ok(Json(prices))
}

async fn add_model_pricing(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(price): Json<pricing::ModelPrice>,
) -> Result<Json<Vec<pricing::ModelPrice>>> {
    let user = auth::require_super_admin(&state, &headers).await?;
    let metadata = json!({"pattern":&price.pattern,"input_per_1m":price.input_per_1m,"output_per_1m":price.output_per_1m,"cached_read_per_1m":price.cached_read_per_1m});
    let prices = pricing::add(&state.db, price).await?;
    pricing::refresh(&state).await;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "platform.model_price_added",
        "model_pricing",
        user.user_id,
        metadata,
    )
    .await;
    Ok(Json(prices))
}

async fn get_unpriced_models(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<pricing::UnpricedModels>> {
    auth::require_super_admin(&state, &headers).await?;
    Ok(Json(pricing::recent_unpriced(&state.db).await?))
}

async fn preview_model_pricing_sync(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<pricing::PriceSyncChange>>> {
    auth::require_super_admin(&state, &headers).await?;
    let fetched = pricing::fetch_openrouter().await.map_err(|_| {
        AppError::ServiceUnavailable("参考价获取失败，请稍后重试，当前价格未更改".into())
    })?;
    Ok(Json(pricing::preview_sync(&state.db, &fetched).await?))
}

#[derive(Deserialize)]
struct PriceSyncInput {
    changes: Vec<pricing::PriceSyncChange>,
}

async fn sync_model_pricing(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<PriceSyncInput>,
) -> Result<Json<Value>> {
    let user = auth::require_super_admin(&state, &headers).await?;
    pricing::validate_sync_changes(&input.changes)?;
    let fetched = pricing::fetch_openrouter().await.map_err(|_| {
        AppError::ServiceUnavailable("参考价获取失败，请稍后重试，当前价格未更改".into())
    })?;
    let synced = pricing::apply_sync(&state.db, &input.changes, &fetched).await?;
    pricing::refresh(&state).await;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "platform.model_pricing_synced",
        "model_pricing",
        user.user_id,
        json!({"requested":input.changes.len(),"synced":synced,"source":"openrouter",
            "patterns":input.changes.iter().take(100).map(|change| &change.pattern).collect::<Vec<_>>()}),
    )
    .await;
    Ok(Json(json!({"synced":synced})))
}

#[derive(Deserialize)]
struct SyncOneInput {
    pattern: String,
}

async fn sync_one_model_pricing(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<SyncOneInput>,
) -> Result<Json<Value>> {
    let user = auth::require_super_admin(&state, &headers).await?;
    let pattern = input.pattern.trim().to_owned();
    if !model_rules::concrete(&pattern) {
        return Err(AppError::BadRequest("只能同步具体模型的价格".into()));
    }
    let exists = || async {
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM model_catalog WHERE pattern=$1)")
            .bind(&pattern)
            .fetch_one(&state.db)
            .await
    };
    if !exists().await? {
        return Err(AppError::NotFound);
    }
    let fetched = pricing::fetch_openrouter().await.map_err(|_| {
        AppError::ServiceUnavailable("参考价获取失败，请稍后重试，当前价格未更改".into())
    })?;
    let updated = pricing::sync_one(&state.db, &fetched, &pattern).await?;
    if updated == 0 {
        // The row may have been deleted while the reference prices were fetched.
        if !exists().await? {
            return Err(AppError::NotFound);
        }
        return Err(AppError::BadRequest(
            "暂无完整的同名参考价，当前价格未更改，可手动设置".into(),
        ));
    }
    pricing::refresh(&state).await;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "platform.model_price_synced",
        "model_pricing",
        user.user_id,
        json!({"pattern":&pattern,"source":"openrouter"}),
    )
    .await;
    Ok(Json(json!({"synced":true})))
}

async fn get_model_pricing_catalog(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<pricing::ModelPrice>>> {
    auth::require_super_admin(&state, &headers).await?;
    let models = pricing::fetch_openrouter()
        .await
        .map_err(|e| AppError::Internal(format!("OpenRouter fetch failed: {e}")))?;
    Ok(Json(models))
}

async fn list_model_catalog(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ModelCatalogView>>> {
    let user = auth::current_user(&state, &headers).await?;
    let query = if user.is_super_admin {
        "SELECT id,pattern,label,enabled,created_at,updated_at,provider,protocols,input_per_1m,output_per_1m,cached_read_per_1m,price_source FROM model_catalog WHERE strpos(pattern,'*')=0 ORDER BY label,pattern"
    } else {
        "SELECT id,pattern,label,enabled,created_at,updated_at,provider,protocols,input_per_1m,output_per_1m,cached_read_per_1m,price_source FROM model_catalog WHERE enabled=true AND strpos(pattern,'*')=0 ORDER BY label,pattern"
    };
    Ok(Json(
        sqlx::query_as::<_, ModelCatalogView>(query)
            .fetch_all(&state.db)
            .await?,
    ))
}

fn validate_catalog_input(input: &ModelCatalogInput) -> Result<()> {
    // Prefix rules belong to channels and keys; the catalog only names models.
    if input.pattern.contains('*') {
        return Err(AppError::BadRequest(
            "模型目录只收录具体模型 ID；前缀规则请在渠道或 API Key 中填写".into(),
        ));
    }
    if let Some(protocols) = &input.protocols {
        if protocols.is_empty() {
            return Err(AppError::BadRequest("至少选择一种调用协议".into()));
        }
        for protocol in protocols {
            validate_protocol(protocol)?;
        }
    }
    if input
        .provider
        .as_ref()
        .is_some_and(|value| value.len() > 100)
    {
        return Err(AppError::BadRequest("厂商名称过长".into()));
    }
    if input.price.is_some() && input.clear_price {
        return Err(AppError::BadRequest("不能同时设置价格和清除价格".into()));
    }
    if let Some(price) = &input.price {
        pricing::validate(price)?;
        if price.pattern != input.pattern.trim() {
            return Err(AppError::BadRequest("价格必须绑定当前具体模型".into()));
        }
    }
    Ok(())
}

async fn create_model_catalog(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<ModelCatalogInput>,
) -> Result<Json<ModelCatalogView>> {
    let user = auth::require_super_admin(&state, &headers).await?;
    validate_catalog_input(&input)?;
    let pattern = normalize_model_pattern(&input.pattern)?;
    let label = validate_model_catalog_label(&input.label)?;
    let row = sqlx::query_as::<_, ModelCatalogView>(
        "INSERT INTO model_catalog(pattern,label,enabled,provider,protocols,input_per_1m,output_per_1m,cached_read_per_1m,price_source) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) RETURNING id,pattern,label,enabled,created_at,updated_at,provider,protocols,input_per_1m,output_per_1m,cached_read_per_1m,price_source",
    )
    .bind(pattern)
    .bind(label)
    .bind(input.enabled.unwrap_or(true))
    .bind(input.provider.as_deref().unwrap_or(""))
    .bind(input.protocols.unwrap_or_else(|| vec!["openai".into(),"anthropic".into(),"gemini".into()]))
    .bind(input.price.as_ref().map(|p|p.input_per_1m))
    .bind(input.price.as_ref().map(|p|p.output_per_1m))
    .bind(input.price.as_ref().map(|p|p.cached_read_per_1m))
    .bind(input.price.as_ref().and_then(|p|p.synced_from.as_deref()))
    .fetch_one(&state.db)
    .await
    .map_err(|error| {
        if error.to_string().contains("duplicate") {
            AppError::Conflict("model pattern already exists".into())
        } else {
            error.into()
        }
    })?;
    pricing::refresh(&state).await;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "model_catalog.created",
        "model_catalog",
        row.id,
        json!({"pattern":row.pattern}),
    )
    .await;
    Ok(Json(row))
}

async fn update_model_catalog(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<ModelCatalogInput>,
) -> Result<Json<ModelCatalogView>> {
    let user = auth::require_super_admin(&state, &headers).await?;
    validate_catalog_input(&input)?;
    let pattern = normalize_model_pattern(&input.pattern)?;
    let label = validate_model_catalog_label(&input.label)?;
    let row = sqlx::query_as::<_, ModelCatalogView>(
        "UPDATE model_catalog SET label=CASE WHEN $12 THEN label ELSE $2 END,enabled=COALESCE($3,enabled),provider=COALESCE($5,provider),protocols=COALESCE($6,protocols),
         input_per_1m=CASE WHEN $10 THEN NULL ELSE COALESCE($7,input_per_1m) END,
         output_per_1m=CASE WHEN $10 THEN NULL ELSE COALESCE($8,output_per_1m) END,
         cached_read_per_1m=CASE WHEN $10 THEN NULL ELSE COALESCE($9,cached_read_per_1m) END,
         price_source=CASE WHEN $10 THEN NULL WHEN $7 IS NOT NULL AND (input_per_1m,output_per_1m,cached_read_per_1m) IS DISTINCT FROM ($7,$8,$9) THEN $11 ELSE price_source END,updated_at=now() WHERE id=$4 AND pattern=$1 AND (NOT $12 OR input_per_1m IS NULL) RETURNING id,pattern,label,enabled,created_at,updated_at,provider,protocols,input_per_1m,output_per_1m,cached_read_per_1m,price_source",
    )
    .bind(pattern)
    .bind(label)
    .bind(input.enabled)
    .bind(id)
    .bind(input.provider)
    .bind(input.protocols)
    .bind(input.price.as_ref().map(|p|p.input_per_1m))
    .bind(input.price.as_ref().map(|p|p.output_per_1m))
    .bind(input.price.as_ref().map(|p|p.cached_read_per_1m))
    .bind(input.clear_price)
    .bind(input.price.as_ref().and_then(|p| p.synced_from.as_deref()))
    .bind(input.only_unpriced)
    .fetch_optional(&state.db)
    .await
    .map_err(|error| {
        if error.to_string().contains("duplicate") {
            AppError::Conflict("model pattern already exists".into())
        } else {
            error.into()
        }
    })?;
    let Some(row) = row else {
        // Explain why nothing was updated: missing row, changed ID or a concurrent price.
        let current: Option<String> =
            sqlx::query_scalar("SELECT pattern FROM model_catalog WHERE id=$1")
                .bind(id)
                .fetch_optional(&state.db)
                .await?;
        return Err(match current {
            None => AppError::NotFound,
            Some(current) if current != input.pattern.trim() => {
                AppError::BadRequest("模型 ID 创建后不可修改".into())
            }
            Some(_) => AppError::Conflict("模型已修改，请刷新后重试".into()),
        });
    };
    pricing::refresh(&state).await;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "model_catalog.updated",
        "model_catalog",
        id,
        json!({"pattern":row.pattern}),
    )
    .await;
    Ok(Json(row))
}

async fn delete_model_catalog(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let user = auth::require_super_admin(&state, &headers).await?;
    let result = sqlx::query("DELETE FROM model_catalog WHERE id=$1")
        .bind(id)
        .execute(&state.db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    pricing::refresh(&state).await;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "model_catalog.deleted",
        "model_catalog",
        id,
        json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

const CHANNEL_CONCURRENCY_RANGE: std::ops::RangeInclusive<i32> = 1..=10000;

fn validate_channel_concurrency(limit: i32) -> Result<()> {
    if CHANNEL_CONCURRENCY_RANGE.contains(&limit) {
        return Ok(());
    }
    Err(AppError::BadRequest(format!(
        "渠道并发上限需在 {} 到 {} 之间",
        CHANNEL_CONCURRENCY_RANGE.start(),
        CHANNEL_CONCURRENCY_RANGE.end()
    )))
}

#[derive(Serialize)]
struct ChannelConcurrency {
    channel_id: Uuid,
    in_use: usize,
    limit: i32,
}

/// Requests in flight now through each of the account's capped channels,
/// including those relayed in from other accounts' keys.
async fn channel_concurrency(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ChannelConcurrency>>> {
    let user = auth::current_user(&state, &headers).await?;
    let rows: Vec<(Uuid, i32)> = sqlx::query_as(
        "SELECT id,concurrency_limit FROM channels WHERE account_id=$1 AND concurrency_limit IS NOT NULL",
    )
    .bind(user.account_id)
    .fetch_all(&state.db)
    .await?;
    let mut result = Vec::with_capacity(rows.len());
    for (channel_id, limit) in rows {
        result.push(ChannelConcurrency {
            channel_id,
            in_use: state.in_use(channel_id).await,
            limit,
        });
    }
    Ok(Json(result))
}

async fn list_channels(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ChannelView>>> {
    let user = auth::current_user(&state, &headers).await?;
    let mut rows = sqlx::query_as::<_, ChannelView>(&format!("SELECT c.id,c.name,c.protocol,c.base_url,c.enabled,c.priority,c.concurrency_limit,CASE WHEN h.status='unhealthy' THEN 'unhealthy' ELSE COALESCE(h.last_observation,h.status,'unknown') END AS health_status,h.last_error AS health_error,h.last_error_kind AS health_error_kind,h.check_note AS health_note,h.checked_at AS health_checked_at,c.test_model,c.models,COALESCE(legacy.mapping,'{{}}'::jsonb) || c.model_mapping AS model_mapping,c.model_mode,c.discovered_models,c.models_synced_at,0::int AS model_count,c.created_at,c.kind,c.provider,c.source_key_id,
         CASE WHEN c.kind='vair_key' THEN COALESCE(({SOURCE_KEY_VIEW} WHERE v.id=c.source_key_id), '{{\"status\":\"revoked\"}}'::jsonb) END AS source
         FROM channels c LEFT JOIN channel_health h ON h.channel_id=c.id LEFT JOIN LATERAL (SELECT jsonb_object_agg(public_model,upstream_model) AS mapping FROM (SELECT DISTINCT ON (public_model) public_model,upstream_model FROM model_routes WHERE channel_id=c.id AND account_id=$1 AND protocol=c.protocol AND enabled=true ORDER BY public_model,priority,id) selected) legacy ON true WHERE c.account_id=$1 GROUP BY c.id,c.test_model,h.status,h.last_observation,h.last_error,h.last_error_kind,h.check_note,h.checked_at,legacy.mapping ORDER BY c.priority,c.created_at"))
        .bind(user.account_id).fetch_all(&state.db).await?;
    let directory = models::directory(&state.db).await?;
    let scopes = models::scopes(&state.db, user.account_id).await?;
    for row in &mut rows {
        if let Some(scope) = scopes.iter().find(|scope| scope.id == row.id) {
            row.known_models = models::channel_names(&directory, scope);
            row.model_count = row.known_models.len() as i32;
        }
        // Models behind a VAIR key change with the owner's channels, so resolve them live.
        if let (Some(key_id), Some(source)) = (row.source_key_id, row.source.as_mut())
            && source["status"] != "revoked"
            && source["status"] != "paused"
        {
            let models = gateway::source_key_model_names(&state, key_id, user.account_id).await?;
            row.model_count = models.len() as i32;
            source["models"] = json!(models);
        }
    }
    Ok(Json(rows))
}

/// What someone given a key may see about it: owner, label, limits, usage in
/// the gateway's limit windows, and a status that says whether the key can
/// serve requests now. Callers append a `WHERE` on `v`; used for the key behind
/// a VAIR key channel and for the key holder's own page.
pub(crate) const SOURCE_KEY_VIEW: &str = "SELECT jsonb_build_object(
      'account_name',a.name,'key_name',v.name,'key_prefix',v.key_prefix,
      'allowed_models',v.allowed_models,'model_mode',v.model_mode,
      'requests_per_minute',v.requests_per_minute,'concurrency_limit',v.concurrency_limit,
      'token_limit_daily',v.token_limit_daily,'cost_limit_daily_usd',v.cost_limit_daily_usd,
      'cost_limit_total_usd',v.cost_limit_total_usd,
      'used_tokens_today',COALESCE(u.used_tokens_today,0),
      'used_cost_today_usd',COALESCE(u.used_cost_today_usd,0),
      'used_cost_total_usd',COALESCE(u.used_cost_total_usd,0),
      'day_started_at',w.day_start,
      'day_ends_at',date_trunc('day',now())+interval '1 day',
      'status',CASE
        WHEN v.revoked_at IS NOT NULL THEN 'revoked'
        WHEN NOT v.enabled OR a.disabled_at IS NOT NULL THEN 'paused'
        WHEN v.cost_limit_total_usd IS NOT NULL AND COALESCE(u.used_cost_total_usd,0)>=v.cost_limit_total_usd THEN 'exhausted_total'
        WHEN (v.token_limit_daily IS NOT NULL AND COALESCE(u.used_tokens_today,0)>=v.token_limit_daily)
          OR (v.cost_limit_daily_usd IS NOT NULL AND COALESCE(u.used_cost_today_usd,0)>=v.cost_limit_daily_usd) THEN 'exhausted_daily'
        ELSE 'active' END)
    FROM virtual_api_keys v JOIN accounts a ON a.id=v.account_id
    CROSS JOIN LATERAL (SELECT GREATEST(date_trunc('day',now()),COALESCE(v.usage_reset_daily_at,'-infinity'::timestamptz)) AS day_start) w
    LEFT JOIN LATERAL (
      SELECT
        SUM(COALESCE(e.input_tokens,0)+COALESCE(e.output_tokens,0)) FILTER(WHERE e.created_at>=w.day_start) AS used_tokens_today,
        SUM(e.cost_usd) FILTER(WHERE e.created_at>=w.day_start) AS used_cost_today_usd,
        SUM(e.cost_usd) AS used_cost_total_usd
      FROM usage_events e
      WHERE e.account_id=v.account_id AND e.virtual_key_id=v.id
        AND e.created_at>=COALESCE(v.usage_reset_total_at,'-infinity'::timestamptz)
    ) u ON true";

async fn preview_channel_secret(
    state: &AppState,
    account_id: Uuid,
    input: &PreviewChannelModelsInput,
) -> Result<String> {
    validate_protocol(&input.protocol)?;
    validate_base_url(&input.base_url)?;
    let stored = if let Some(id) = input.channel_id {
        Some(sqlx::query("SELECT ck.secret_ciphertext FROM channels c LEFT JOIN channel_keys ck ON ck.channel_id=c.id AND ck.enabled=true WHERE c.id=$1 AND c.account_id=$2 ORDER BY ck.created_at,ck.id LIMIT 1")
            .bind(id).bind(account_id).fetch_optional(&state.db).await?.ok_or(AppError::NotFound)?)
    } else {
        None
    };
    if let Some(secret) = input
        .secret
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if secret.len() > 4096 {
            return Err(AppError::BadRequest("请输入有效的上游 API Key。".into()));
        }
        return Ok(secret.to_owned());
    }
    let ciphertext = stored
        .map(|row| row.try_get::<Option<String>, _>("secret_ciphertext"))
        .transpose()?
        .flatten()
        .ok_or_else(|| AppError::BadRequest("未配置可用的上游 API Key，请在表单中填写。".into()))?;
    state
        .crypto
        .decrypt(&ciphertext)
        .map_err(|_| AppError::BadRequest("上游 API Key 无法读取，请在表单中重新填写。".into()))
}

async fn preview_channel_models(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<PreviewChannelModelsInput>,
) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    let secret = preview_channel_secret(&state, user.account_id, &input).await?;
    let models =
        gateway::fetch_models_from_config(&state, &input.protocol, input.base_url.trim(), &secret)
            .await?;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "channel.models_previewed",
        "account",
        user.account_id,
        json!({"protocol": input.protocol, "count": models.len()}),
    )
    .await;
    Ok(Json(json!({"models": models})))
}

async fn preview_channel_check(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<PreviewChannelCheckInput>,
) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    let secret = preview_channel_secret(&state, user.account_id, &input.connection).await?;
    let model = input.model.trim();
    if model.is_empty() || model.len() > 256 || model.chars().any(char::is_control) {
        return Err(AppError::BadRequest("请输入有效的测试模型".into()));
    }
    let result = gateway::probe_channel_config(
        &state,
        &input.connection.protocol,
        input.connection.base_url.trim(),
        &secret,
        model,
    )
    .await?;
    // A rate-limited upstream accepted the address and key; it is only busy.
    let rate_limited = result.0 == gateway::CheckKind::RateLimited;
    let passed = rate_limited || result.0 == gateway::CheckKind::Healthy;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "channel.connection_previewed",
        "account",
        user.account_id,
        json!({"protocol": input.connection.protocol, "healthy": passed, "status": result.0.status()}),
    )
    .await;
    let receipt = if passed {
        Some(
            channel_preview::CheckConfig {
                account_id: user.account_id,
                channel_id: input.connection.channel_id,
                protocol: &input.connection.protocol,
                base_url: &input.connection.base_url,
                secret: &secret,
                model,
            }
            .receipt(&state.crypto, result.2, rate_limited)?,
        )
    } else {
        None
    };
    Ok(Json(
        json!({"healthy": passed, "rate_limited": rate_limited, "message": result.1, "check_receipt": receipt}),
    ))
}

async fn check_channel(
    State(state): State<AppState>,
    Path(channel_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    let result = gateway::probe_channel(&state, user.account_id, channel_id).await?;
    Ok(Json(
        json!({"channel_id":channel_id,"healthy":result.0,"message":result.1}),
    ))
}

async fn load_monitoring_channel(
    db: &PgPool,
    account_id: Uuid,
    channel_id: Option<Uuid>,
) -> Result<Vec<MonitoringChannelView>> {
    let mut query = String::from(
        "SELECT c.id,c.name,c.protocol,c.kind,c.enabled,c.priority,c.models,c.test_model,
                -- An unavailable channel stays shown as such; otherwise the
                -- last check that did not conclude says what it saw.
                CASE WHEN h.status='unhealthy' THEN 'unhealthy' ELSE COALESCE(h.last_observation,h.status,'unknown') END AS availability_status,
                h.last_observation AS availability_observation,
                h.last_error AS availability_error,h.last_error_kind AS availability_error_kind,h.check_note AS availability_note,
                h.checked_at AS availability_checked_at,
                COALESCE(mc.availability_enabled,true) AS availability_enabled,
                COALESCE(mc.availability_interval_seconds,1800) AS availability_interval_seconds,
                COALESCE(h.consecutive_failures,0) AS availability_failures,
                COALESCE(mc.availability_failure_threshold,2) AS availability_failure_threshold,
                COALESCE(mc.availability_route_policy,'exclude') AS availability_route_policy,
                c.first_token_timeout_seconds
         FROM channels c
         LEFT JOIN channel_health h ON h.channel_id=c.id
         LEFT JOIN channel_monitor_configs mc ON mc.channel_id=c.id
         WHERE c.account_id=$1",
    );
    if channel_id.is_some() {
        query.push_str(" AND c.id=$2");
    }
    query.push_str(" ORDER BY c.priority,c.created_at");
    let mut request = sqlx::query_as::<_, MonitoringChannelView>(&query).bind(account_id);
    if let Some(channel_id) = channel_id {
        request = request.bind(channel_id);
    }
    Ok(request.fetch_all(db).await?)
}

async fn monitoring_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<MonitoringSummaryView>> {
    let user = auth::current_user(&state, &headers).await?;
    let channels = load_monitoring_channel(&state.db, user.account_id, None).await?;
    let available_channels = channels
        .iter()
        .filter(|channel| {
            channel.enabled
                && channel.availability_enabled
                && channel.availability_status == "healthy"
        })
        .count();
    Ok(Json(MonitoringSummaryView {
        total_channels: channels.len(),
        available_channels,
        channels,
    }))
}

async fn get_channel_monitor(
    State(state): State<AppState>,
    Path(channel_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<MonitoringChannelView>> {
    let user = auth::current_user(&state, &headers).await?;
    Ok(Json(
        load_monitoring_channel(&state.db, user.account_id, Some(channel_id))
            .await?
            .into_iter()
            .next()
            .ok_or(AppError::NotFound)?,
    ))
}

async fn update_channel_monitor(
    State(state): State<AppState>,
    Path(channel_id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<UpdateMonitorInput>,
) -> Result<Json<MonitoringChannelView>> {
    let user = auth::current_user(&state, &headers).await?;
    if input
        .availability_interval_seconds
        .is_some_and(|n| !(300..=86400).contains(&n))
    {
        return Err(AppError::BadRequest(
            "检查频率必须在 5 分钟至 24 小时之间".into(),
        ));
    }
    if input
        .availability_failure_threshold
        .is_some_and(|n| !(1..=5).contains(&n))
    {
        return Err(AppError::BadRequest(
            "判定异常的连续失败次数必须在 1 至 5 次之间".into(),
        ));
    }
    if input
        .availability_route_policy
        .as_deref()
        .is_some_and(|policy| policy != "exclude" && policy != "observe")
    {
        return Err(AppError::BadRequest("判定异常后的处理方式无效".into()));
    }
    if input
        .first_token_timeout_seconds
        .is_some_and(|n| !(5..=300).contains(&n))
    {
        return Err(AppError::BadRequest(
            "流式首字超时必须在 5 至 300 秒之间".into(),
        ));
    }
    let mut transaction = state.db.begin().await?;
    let changed = sqlx::query(
        "UPDATE channel_monitor_configs mc SET availability_enabled=COALESCE($3,availability_enabled),
         availability_interval_seconds=COALESCE($4,availability_interval_seconds),
         availability_failure_threshold=COALESCE($5,availability_failure_threshold),
         availability_route_policy=COALESCE($6,availability_route_policy),
         next_availability_at=CASE WHEN COALESCE($3,availability_enabled) THEN now() ELSE NULL END,updated_at=now()
         FROM channels c WHERE mc.channel_id=c.id AND c.id=$1 AND c.account_id=$2")
        .bind(channel_id).bind(user.account_id).bind(input.availability_enabled).bind(input.availability_interval_seconds)
        .bind(input.availability_failure_threshold).bind(&input.availability_route_policy)
        .execute(&mut *transaction).await?;
    if changed.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    if let Some(seconds) = input.first_token_timeout_seconds {
        sqlx::query(
            "UPDATE channels SET first_token_timeout_seconds=$3 WHERE id=$1 AND account_id=$2",
        )
        .bind(channel_id)
        .bind(user.account_id)
        .bind(seconds)
        .execute(&mut *transaction)
        .await?;
    }
    // A lower threshold applies at once to failures already counted.
    sqlx::query(
        "UPDATE channel_health h SET status='unhealthy',unhealthy_since=COALESCE(h.unhealthy_since,now()),updated_at=now() FROM channel_monitor_configs mc
         WHERE h.channel_id=$1 AND mc.channel_id=h.channel_id AND h.status<>'unhealthy'
           AND h.consecutive_failures>=mc.availability_failure_threshold",
    )
    .bind(channel_id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    audit(&state.db,user.account_id,user.user_id,"channel.monitor_updated","channel",channel_id,
          json!({"availability_enabled":input.availability_enabled,"availability_interval_seconds":input.availability_interval_seconds,
                 "availability_failure_threshold":input.availability_failure_threshold,"availability_route_policy":input.availability_route_policy,
                 "first_token_timeout_seconds":input.first_token_timeout_seconds})).await;
    Ok(Json(
        load_monitoring_channel(&state.db, user.account_id, Some(channel_id))
            .await?
            .remove(0),
    ))
}

async fn monitoring_runs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<MonitorRunsQuery>,
) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    let kind = query.kind.map(MonitorKind::kinds);
    if let Some(id) = query.monitor_id {
        quality_monitors::ensure_owned(&state.db, user.account_id, id, true).await?;
    }
    if let Some(per_channel) = query.per_channel {
        let rows = sqlx::query_as::<_, MonitorRunView>(
            "SELECT id,monitor_id,channel_id,kind,model,status,score,latency_ms,ttfb_ms,status_code,error_code,details,started_at,finished_at
             FROM (
               SELECT r.id,r.monitor_id,r.channel_id,r.kind,r.model,r.status,r.score::double precision AS score,
                      r.latency_ms,r.ttfb_ms,r.status_code,r.error_code,r.details,r.started_at,r.finished_at,
                      r.created_at,
                      ROW_NUMBER() OVER (PARTITION BY r.channel_id,r.kind,r.monitor_id ORDER BY r.created_at DESC,r.id DESC) AS rn
               FROM channel_monitor_runs r JOIN channels c ON c.id=r.channel_id
               WHERE c.account_id=$1 AND ($2::uuid IS NULL OR r.channel_id=$2)
                 AND ($3::text[] IS NULL OR r.kind=ANY($3)) AND ($5::uuid IS NULL OR r.monitor_id=$5)
             ) ranked
             WHERE rn <= $4
             ORDER BY created_at DESC,id DESC",
        )
        .bind(user.account_id)
        .bind(query.channel_id)
        .bind(&kind)
        .bind(per_channel.clamp(1, 50))
        .bind(query.monitor_id)
        .fetch_all(&state.db)
        .await?;
        return Ok(Json(json!(rows)));
    }
    let limit = query.limit.unwrap_or(72).clamp(1, 200);
    let total = if query.page.is_some() {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM channel_monitor_runs r JOIN channels c ON c.id=r.channel_id
             WHERE c.account_id=$1 AND ($2::uuid IS NULL OR r.channel_id=$2)
               AND ($3::text[] IS NULL OR r.kind=ANY($3)) AND ($4::uuid IS NULL OR r.monitor_id=$4)",
        )
        .bind(user.account_id)
        .bind(query.channel_id)
        .bind(&kind)
        .bind(query.monitor_id)
        .fetch_one(&state.db)
        .await?
    } else {
        0
    };
    let page = query
        .page
        .unwrap_or(1)
        .clamp(1, ((total - 1) / limit + 1).max(1));
    let rows = sqlx::query_as::<_, MonitorRunView>(
        "SELECT r.id,r.monitor_id,r.channel_id,r.kind,r.model,r.status,r.score::double precision AS score,
                r.latency_ms,r.ttfb_ms,r.status_code,r.error_code,r.details,r.started_at,r.finished_at
         FROM channel_monitor_runs r JOIN channels c ON c.id=r.channel_id
         WHERE c.account_id=$1 AND ($2::uuid IS NULL OR r.channel_id=$2)
           AND ($3::text[] IS NULL OR r.kind=ANY($3)) AND ($6::uuid IS NULL OR r.monitor_id=$6)
         ORDER BY r.created_at DESC,r.id DESC LIMIT $4 OFFSET $5",
    )
    .bind(user.account_id)
    .bind(query.channel_id)
    .bind(&kind)
    .bind(limit)
    .bind((page - 1) * limit)
    .bind(query.monitor_id)
    .fetch_all(&state.db)
    .await?;
    // Keep the recent-runs response compatible with heatmap consumers.
    Ok(Json(if query.page.is_some() {
        json!({"items":rows,"total":total,"page":page,"page_size":limit})
    } else {
        json!(rows)
    }))
}

async fn sync_channel_models(
    State(state): State<AppState>,
    Path(channel_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    vair_key_channels::ensure_upstream_channel(&state.db, user.account_id, channel_id).await?;
    let models = gateway::fetch_channel_models(&state, user.account_id, channel_id).await?;
    sqlx::query(
        "UPDATE channels SET discovered_models=$1,models_synced_at=now(),updated_at=now() WHERE id=$2 AND account_id=$3",
    )
    .bind(json!(&models))
    .bind(channel_id)
    .bind(user.account_id)
    .execute(&state.db)
    .await?;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "channel.models_synced",
        "channel",
        channel_id,
        json!({"count": models.len()}),
    )
    .await;
    Ok(Json(json!({"channel_id": channel_id, "models": models})))
}

fn validate_channel_scope(mode: &str, rules: &[String], test_model: Option<&str>) -> Result<()> {
    model_rules::validate_mode(mode, "passthrough", rules)?;
    if (mode == "passthrough" || !rules.iter().any(|rule| model_rules::concrete(rule)))
        && !test_model.is_some_and(model_rules::concrete)
    {
        return Err(AppError::BadRequest(
            "全部转发或仅前缀规则的渠道必须填写具体测试模型".into(),
        ));
    }
    Ok(())
}

async fn update_channel_models(
    state: State<AppState>,
    channel_id: Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<UpdateChannelModelsInput>,
) -> Result<Json<Value>> {
    update_channel(
        state,
        channel_id,
        headers,
        Json(UpdateChannelInput {
            name: None,
            base_url: None,
            priority: None,
            models: Some(input.models),
            model_mapping: None,
            model_mode: input.model_mode,
            test_model: input.test_model,
            enabled: None,
            secret: None,
            check_receipt: None,
            concurrency_limit: None,
        }),
    )
    .await
}

async fn update_channel(
    State(state): State<AppState>,
    Path(channel_id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<UpdateChannelInput>,
) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    // A VAIR key channel has no upstream URL, credential or model rules of its
    // own; its test model may still be set for checks through the source key.
    if input.base_url.is_some()
        || input.models.is_some()
        || input.model_mode.is_some()
        || input.secret.is_some()
        || input.check_receipt.is_some()
        || input.concurrency_limit.is_some()
    {
        vair_key_channels::ensure_upstream_channel(&state.db, user.account_id, channel_id).await?;
    }
    let name = input.name.map(|value| value.trim().to_owned());
    if name
        .as_deref()
        .is_some_and(|value| value.is_empty() || value.len() > 120)
    {
        return Err(AppError::BadRequest("channel name is required".into()));
    }
    let base_url = input.base_url.map(|value| value.trim().to_owned());
    if let Some(value) = &base_url {
        validate_base_url(value)?;
    }
    let test_model = input
        .test_model
        .map(|value| value.map(|s| s.trim().to_owned()));
    if test_model
        .as_ref()
        .and_then(|v| v.as_deref())
        .is_some_and(|value| !model_rules::concrete(value))
    {
        return Err(AppError::BadRequest("test model is invalid".into()));
    }
    let models = input.models.map(normalize_channel_models).transpose()?;
    let model_mode = input.model_mode;
    let mut transaction = state.db.begin().await?;
    // Compare against the mapping the editor shows, which includes legacy routes.
    // NO KEY UPDATE still lets monitor runs reference the channel by foreign key.
    let current = sqlx::query("SELECT c.kind,c.protocol,c.base_url,c.models,c.model_mode,c.test_model,COALESCE(legacy.mapping,'{}'::jsonb) || c.model_mapping AS model_mapping FROM channels c LEFT JOIN LATERAL (SELECT jsonb_object_agg(public_model,upstream_model) AS mapping FROM (SELECT DISTINCT ON (public_model) public_model,upstream_model FROM model_routes WHERE channel_id=c.id AND account_id=$2 AND protocol=c.protocol AND enabled=true ORDER BY public_model,priority,id) selected) legacy ON true WHERE c.id=$1 AND c.account_id=$2 FOR NO KEY UPDATE OF c")
        .bind(channel_id).bind(user.account_id).fetch_optional(&mut *transaction).await?
        .ok_or(AppError::NotFound)?;
    let current_rules: Vec<String> = serde_json::from_value(current.try_get("models")?)
        .map_err(|_| AppError::Internal("invalid channel rules".into()))?;
    let current_mode: String = current.try_get("model_mode")?;
    let current_test: Option<String> = current.try_get("test_model")?;
    let current_base: String = current.try_get("base_url")?;
    let current_mapping: Value = current.try_get("model_mapping")?;
    let final_test = test_model.as_ref().unwrap_or(&current_test).as_deref();
    let final_rules = models.as_deref().unwrap_or(&current_rules);
    // A VAIR key channel's test model is optional: checks default to a source-key model.
    if (models.is_some() || model_mode.is_some() || test_model.is_some())
        && current.try_get::<String, _>("kind")? == "upstream"
    {
        validate_channel_scope(
            model_mode.as_deref().unwrap_or(&current_mode),
            final_rules,
            final_test,
        )?;
    }
    if input.priority.is_some_and(|priority| priority < 0) {
        return Err(AppError::BadRequest("priority must be non-negative".into()));
    }
    if let Some(Some(limit)) = input.concurrency_limit {
        validate_channel_concurrency(limit)?;
    }
    let mapping = input
        .model_mapping
        .map(normalize_model_mapping)
        .transpose()?;
    let encrypted = input
        .secret
        .as_deref()
        .map(|secret| {
            let secret = secret.trim();
            if secret.is_empty() || secret.len() > 4096 {
                return Err(AppError::BadRequest("请输入有效的上游 API Key。".into()));
            }
            state.crypto.encrypt(secret).map_err(AppError::Internal)
        })
        .transpose()?;
    let connection_changed =
        encrypted.is_some() || base_url.as_ref().is_some_and(|url| url != &current_base);
    let check_changed = connection_changed
        || test_model
            .as_ref()
            .is_some_and(|test| test != &current_test)
        || models.as_ref().is_some_and(|rules| rules != &current_rules)
        || mapping
            .as_ref()
            .is_some_and(|value| value != &current_mapping);
    let result = sqlx::query("UPDATE channels SET name=COALESCE($1,name),base_url=COALESCE($2,base_url),priority=COALESCE($3,priority),models=COALESCE($4,models),model_mode=COALESCE($5,model_mode),test_model=CASE WHEN $11 THEN $8 ELSE test_model END,model_mapping=COALESCE($9,model_mapping),enabled=COALESCE($10,enabled),concurrency_limit=CASE WHEN $12 THEN $13 ELSE concurrency_limit END,updated_at=now() WHERE id=$6 AND account_id=$7")
        .bind(name)
        .bind(&base_url)
        .bind(input.priority)
        .bind(models.as_ref().map(|items| json!(items)))
        .bind(model_mode)
        .bind(channel_id)
        .bind(user.account_id)
        .bind(final_test)
        .bind(&mapping)
        .bind(input.enabled)
        .bind(test_model.is_some())
        .bind(input.concurrency_limit.is_some())
        .bind(input.concurrency_limit.flatten())
        .execute(&mut *transaction)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    if let Some(ciphertext) = &encrypted {
        // The account-scoped channel update above locks the parent row, serializing
        // credential changes even when the channel has no key yet.
        let key_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM channel_keys WHERE channel_id=$1 ORDER BY enabled DESC,created_at,id LIMIT 1 FOR UPDATE",
        )
        .bind(channel_id)
        .fetch_optional(&mut *transaction)
        .await?;
        if let Some(key_id) = key_id {
            sqlx::query("UPDATE channel_keys SET secret_ciphertext=$3,enabled=true WHERE id=$1 AND channel_id=$2")
                .bind(key_id).bind(channel_id).bind(ciphertext)
                .execute(&mut *transaction).await?;
        } else {
            sqlx::query("INSERT INTO channel_keys(channel_id,label,secret_ciphertext) VALUES ($1,'default',$2)")
                .bind(channel_id).bind(ciphertext)
                .execute(&mut *transaction).await?;
        }
    }
    if connection_changed {
        sqlx::query("UPDATE channels SET discovered_models='[]',models_synced_at=NULL WHERE id=$1 AND account_id=$2")
            .bind(channel_id).bind(user.account_id).execute(&mut *transaction).await?;
    }
    if check_changed {
        sqlx::query("UPDATE channel_health SET status='unknown',last_observation=NULL,check_note=NULL,last_error=NULL,last_error_kind=NULL,consecutive_failures=0,failing_since=NULL,last_failure_at=NULL,checked_at=NULL,updated_at=now() WHERE channel_id=$1")
            .bind(channel_id).execute(&mut *transaction).await?;
    }
    if let Some(receipt) = &input.check_receipt {
        let ciphertext: Option<String> = sqlx::query_scalar("SELECT secret_ciphertext FROM channel_keys WHERE channel_id=$1 AND enabled=true ORDER BY created_at,id LIMIT 1")
            .bind(channel_id).fetch_optional(&mut *transaction).await?;
        let secret = ciphertext
            .and_then(|value| state.crypto.decrypt(&value).ok())
            .ok_or_else(|| AppError::BadRequest("上游 API Key 无法读取，请重新填写。".into()))?;
        let protocol: String = current.try_get("protocol")?;
        let model = channel_preview::test_model(
            final_test,
            final_rules,
            mapping.as_ref().unwrap_or(&current_mapping),
        )
        .ok_or_else(|| AppError::BadRequest("请填写具体测试模型".into()))?;
        let check = channel_preview::CheckConfig {
            account_id: user.account_id,
            channel_id: Some(channel_id),
            protocol: &protocol,
            base_url: base_url.as_deref().unwrap_or(&current_base),
            secret: &secret,
            model,
        }
        .verify(&state.crypto, receipt)?;
        channel_preview::save_health(&mut transaction, channel_id, &check).await?;
    }
    // Legacy routes are presented as mappings in the editor. Reconcile them atomically
    // so removing an alias cannot leave a hidden route, while retaining existing priorities.
    if let Some(mapping) = &mapping {
        sqlx::query("DELETE FROM model_routes WHERE channel_id=$1 AND account_id=$2 AND enabled=true AND NOT ($3::jsonb ? public_model)")
            .bind(channel_id).bind(user.account_id).bind(mapping)
            .execute(&mut *transaction).await?;
        sqlx::query("UPDATE model_routes SET upstream_model=$3::jsonb ->> public_model WHERE channel_id=$1 AND account_id=$2 AND enabled=true AND $3::jsonb ? public_model")
            .bind(channel_id).bind(user.account_id).bind(mapping)
            .execute(&mut *transaction).await?;
    }
    transaction.commit().await?;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "channel.updated",
        "channel",
        channel_id,
        json!({"models_updated": models.is_some(), "mapping_updated": mapping.is_some(), "credential_updated": encrypted.is_some(), "enabled": input.enabled, "concurrency_limit": input.concurrency_limit}),
    )
    .await;
    Ok(Json(json!({"channel_id": channel_id})))
}

async fn create_channel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateChannelInput>,
) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    validate_protocol(&input.protocol)?;
    validate_base_url(&input.base_url)?;
    if input
        .provider
        .as_deref()
        .is_some_and(|provider| !providers::supports(provider, &input.protocol))
    {
        return Err(AppError::BadRequest(
            "provider is unknown or does not offer this protocol".into(),
        ));
    }
    let test_model = input.test_model.map(|value| value.trim().to_owned());
    if test_model
        .as_deref()
        .is_some_and(|value| !model_rules::concrete(value))
    {
        return Err(AppError::BadRequest("test model is invalid".into()));
    }
    let models = normalize_channel_models(input.models.unwrap_or_default())?;
    let model_mode = input.model_mode.as_deref().unwrap_or("allowlist");
    validate_channel_scope(model_mode, &models, test_model.as_deref())?;
    if input.name.trim().is_empty() || input.name.len() > 120 {
        return Err(AppError::BadRequest("channel name is required".into()));
    }
    let secret = input.secret.trim();
    if secret.is_empty() || secret.len() > 4096 {
        return Err(AppError::BadRequest("channel key is invalid".into()));
    }
    if input.priority.is_some_and(|priority| priority < 0) {
        return Err(AppError::BadRequest("priority must be non-negative".into()));
    }
    if let Some(limit) = input.concurrency_limit {
        validate_channel_concurrency(limit)?;
    }
    let check = input
        .check_receipt
        .as_deref()
        .map(|receipt| {
            let mapping = json!({});
            let model = channel_preview::test_model(test_model.as_deref(), &models, &mapping)
                .ok_or_else(|| AppError::BadRequest("请填写具体测试模型".into()))?;
            channel_preview::CheckConfig {
                account_id: user.account_id,
                channel_id: None,
                protocol: &input.protocol,
                base_url: &input.base_url,
                secret,
                model,
            }
            .verify(&state.crypto, receipt)
        })
        .transpose()?;
    let limits = load_account_limits(&state.db).await;
    let encrypted = state.crypto.encrypt(secret).map_err(AppError::Internal)?;
    let mut tx = state.db.begin().await?;
    reserve_channel_slot(&mut tx, user.account_id, limits.channel_limit).await?;
    let row = sqlx::query("INSERT INTO channels(account_id,name,protocol,base_url,priority,models,model_mode,test_model,provider,concurrency_limit) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) RETURNING id")
        .bind(user.account_id).bind(input.name.trim()).bind(&input.protocol).bind(input.base_url.trim()).bind(input.priority.unwrap_or(100)).bind(json!(models)).bind(model_mode).bind(test_model).bind(&input.provider).bind(input.concurrency_limit).fetch_one(&mut *tx).await?;
    let channel_id: Uuid = sqlx::Row::try_get(&row, "id")
        .map_err(|_| AppError::Internal("invalid channel row".into()))?;
    sqlx::query("INSERT INTO channel_monitor_configs(channel_id) VALUES ($1) ON CONFLICT(channel_id) DO NOTHING")
        .bind(channel_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO channel_keys(channel_id,label,secret_ciphertext) VALUES ($1,$2,$3)")
        .bind(channel_id)
        .bind("default")
        .bind(encrypted)
        .execute(&mut *tx)
        .await?;
    if let Some(check) = &check {
        channel_preview::save_health(&mut tx, channel_id, check).await?;
    }
    tx.commit().await?;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "channel.created",
        "channel",
        channel_id,
        json!({"protocol":input.protocol,"provider":input.provider,"concurrency_limit":input.concurrency_limit}),
    )
    .await;
    Ok(Json(json!({"id":channel_id})))
}

async fn delete_channel(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let user = auth::current_user(&state, &headers).await?;
    let result = sqlx::query("DELETE FROM channels WHERE id=$1 AND account_id=$2")
        .bind(id)
        .bind(user.account_id)
        .execute(&state.db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    state.forget_semaphore(id).await;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "channel.deleted",
        "channel",
        id,
        json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_routes(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<RouteView>>> {
    let user = auth::current_user(&state, &headers).await?;
    let rows = sqlx::query_as::<_, RouteView>("SELECT r.id,r.channel_id,c.name AS channel_name,r.public_model,r.upstream_model,r.protocol,r.priority,r.enabled FROM model_routes r JOIN channels c ON c.id=r.channel_id WHERE r.account_id=$1 ORDER BY r.public_model,r.priority")
        .bind(user.account_id).fetch_all(&state.db).await?;
    Ok(Json(rows))
}

async fn create_route(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateRouteInput>,
) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    validate_protocol(&input.protocol)?;
    if !model_rules::concrete(input.public_model.trim())
        || !model_rules::concrete(input.upstream_model.trim())
    {
        return Err(AppError::BadRequest("model names are required".into()));
    }
    let matches: Option<String> =
        sqlx::query_scalar("SELECT protocol FROM channels WHERE id=$1 AND account_id=$2")
            .bind(input.channel_id)
            .bind(user.account_id)
            .fetch_optional(&state.db)
            .await?;
    if matches.as_deref() != Some(input.protocol.as_str()) {
        return Err(AppError::BadRequest(
            "route protocol must match channel protocol".into(),
        ));
    }
    let row = sqlx::query("INSERT INTO model_routes(account_id,channel_id,public_model,upstream_model,protocol,priority) VALUES ($1,$2,$3,$4,$5,$6) RETURNING id")
        .bind(user.account_id).bind(input.channel_id).bind(input.public_model.trim()).bind(input.upstream_model.trim()).bind(&input.protocol).bind(input.priority.unwrap_or(100)).fetch_one(&state.db).await?;
    let id: Uuid = sqlx::Row::try_get(&row, "id")
        .map_err(|_| AppError::Internal("invalid route row".into()))?;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "route.created",
        "route",
        id,
        json!({"model":input.public_model}),
    )
    .await;
    Ok(Json(json!({"id":id})))
}

async fn delete_route(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let user = auth::current_user(&state, &headers).await?;
    let result = sqlx::query("DELETE FROM model_routes WHERE id=$1 AND account_id=$2")
        .bind(id)
        .bind(user.account_id)
        .execute(&state.db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "route.deleted",
        "route",
        id,
        json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<VirtualKeyView>>> {
    let user = auth::current_user(&state, &headers).await?;
    Ok(Json(key_views(&state, user.account_id).await?))
}

/// The account's unrevoked keys with usage and the models each can reach,
/// including those reached through VAIR key channels.
async fn key_views(state: &AppState, account_id: Uuid) -> Result<Vec<VirtualKeyView>> {
    let mut rows = sqlx::query_as::<_, VirtualKeyView>(
        "SELECT v.id,v.name,v.holder,v.key_prefix,(v.secret_ciphertext IS NOT NULL) AS can_copy,v.allowed_models,v.model_mode,
           v.requests_per_minute,v.token_limit_daily,v.cost_limit_daily_usd,v.cost_limit_total_usd,
           v.concurrency_limit,v.enabled,v.created_at,v.revoked_at,
           v.usage_reset_daily_at,v.usage_reset_total_at,v.channel_mode,v.group_id,
           ARRAY(SELECT vc.channel_id FROM virtual_key_channels vc WHERE vc.virtual_key_id=v.id) AS channel_ids,
           COALESCE(u.used_tokens_today,0)::bigint AS used_tokens_today,
           COALESCE(u.used_cost_today_usd,0)::float8 AS used_cost_today_usd,
           COALESCE(u.used_cost_total_usd,0)::float8 AS used_cost_total_usd,
           (SELECT COUNT(*) FROM channels c
            WHERE c.source_key_id=v.id) AS channel_count,
           COALESCE((SELECT jsonb_agg(DISTINCT jsonb_build_object('name',a.name,'email',u.email))
            FROM key_share_links l
            JOIN channels c ON c.source_key_id=l.virtual_key_id AND c.account_id=l.accepted_account_id
            JOIN accounts a ON a.id=l.accepted_account_id
            LEFT JOIN memberships m ON m.account_id=a.id
            LEFT JOIN users u ON u.id=m.user_id
            WHERE l.virtual_key_id=v.id AND l.accepted_at IS NOT NULL),'[]'::jsonb) AS shared_accounts,
           (SELECT l.expires_at FROM key_share_links l
            WHERE l.virtual_key_id=v.id AND l.accepted_at IS NULL AND l.cancelled_at IS NULL
              AND l.expires_at>now()) AS share_expires_at
         FROM virtual_api_keys v
         CROSS JOIN LATERAL (SELECT GREATEST(date_trunc('day',now()),COALESCE(v.usage_reset_daily_at,'-infinity'::timestamptz)) AS day_start) w
         LEFT JOIN LATERAL (
           SELECT
             SUM(COALESCE(e.input_tokens,0)+COALESCE(e.output_tokens,0)) FILTER(WHERE e.created_at>=w.day_start) AS used_tokens_today,
             SUM(e.cost_usd) FILTER(WHERE e.created_at>=w.day_start) AS used_cost_today_usd,
             SUM(e.cost_usd) AS used_cost_total_usd
           FROM usage_events e
           WHERE e.account_id=v.account_id AND e.virtual_key_id=v.id
             AND e.created_at>=COALESCE(v.usage_reset_total_at,'-infinity'::timestamptz)
         ) u ON true
         WHERE v.account_id=$1 AND v.revoked_at IS NULL ORDER BY v.created_at DESC",
    )
    .bind(account_id)
    .fetch_all(&state.db)
    .await?;
    for row in &mut rows {
        row.models = gateway::key_models_by_protocol(state, row.id).await?;
        row.known_models = row
            .models
            .values()
            .flatten()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .cloned()
            .collect();
    }
    Ok(rows)
}

/// Selected keys need at least one channel, and every channel must belong to
/// the key's account.
async fn key_channels(
    db: &PgPool,
    account_id: Uuid,
    mode: &str,
    ids: Vec<Uuid>,
) -> Result<Vec<Uuid>> {
    match mode {
        "all" => Ok(Vec::new()),
        "selected" => {
            let ids: Vec<Uuid> = ids
                .into_iter()
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            if ids.is_empty() {
                return Err(AppError::BadRequest(
                    "至少选择一个渠道；如需全部渠道，请勾选允许全部渠道".into(),
                ));
            }
            let owned: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM channels WHERE account_id=$1 AND id=ANY($2)",
            )
            .bind(account_id)
            .bind(&ids)
            .fetch_one(db)
            .await?;
            if owned != ids.len() as i64 {
                return Err(AppError::BadRequest("渠道不存在".into()));
            }
            Ok(ids)
        }
        _ => Err(AppError::BadRequest("invalid channel mode".into())),
    }
}

async fn replace_key_channels(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    key_id: Uuid,
    mode: &str,
    ids: &[Uuid],
) -> Result<()> {
    sqlx::query("UPDATE virtual_api_keys SET channel_mode=$2 WHERE id=$1")
        .bind(key_id)
        .bind(mode)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM virtual_key_channels WHERE virtual_key_id=$1")
        .bind(key_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        "INSERT INTO virtual_key_channels(virtual_key_id,channel_id) SELECT $1, unnest($2::uuid[])",
    )
    .bind(key_id)
    .bind(ids)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// A key's holder, trimmed; empty means none.
fn key_holder_name(holder: Option<&str>) -> Result<Option<String>> {
    let holder = holder.map(str::trim).filter(|holder| !holder.is_empty());
    if holder.is_some_and(|holder| holder.chars().count() > 120) {
        return Err(AppError::BadRequest("持有人不能超过 120 个字符".into()));
    }
    Ok(holder.map(str::to_owned))
}

async fn create_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateVirtualKeyInput>,
) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    // A grouped key takes its scope from the group after the insert.
    let grouped = input.group_id.is_some();
    let allowed_models = if grouped {
        Vec::new()
    } else {
        normalize_allowed_models(input.allowed_models)?
    };
    let model_mode = if grouped {
        "all"
    } else {
        input.model_mode.as_deref().unwrap_or("allowlist")
    };
    model_rules::validate_mode(model_mode, "all", &allowed_models)?;
    let allowed_models = model_rules::key_rules(model_mode, allowed_models);
    if input.requests_per_minute.is_some_and(|v| v <= 0)
        || input.token_limit_daily.is_some_and(|v| v <= 0)
        || input
            .cost_limit_daily_usd
            .is_some_and(|v| !v.is_finite() || v <= 0.0)
        || input
            .cost_limit_total_usd
            .is_some_and(|v| !v.is_finite() || v <= 0.0)
    {
        return Err(AppError::BadRequest("limits must be positive".into()));
    }
    if input.name.trim().is_empty() || input.name.len() > 120 {
        return Err(AppError::BadRequest("key name is required".into()));
    }
    let holder = key_holder_name(input.holder.as_deref())?;
    let channel_mode = if grouped {
        "all"
    } else {
        input.channel_mode.as_deref().unwrap_or("all")
    };
    let channel_ids =
        key_channels(&state.db, user.account_id, channel_mode, input.channel_ids).await?;
    let account = get_account(&state.db, user.account_id).await?;
    let limits = load_account_limits(&state.db).await;
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM virtual_api_keys WHERE account_id=$1 AND revoked_at IS NULL",
    )
    .bind(user.account_id)
    .fetch_one(&state.db)
    .await?;
    if count >= limits.virtual_key_limit {
        return Err(AppError::Conflict(
            "account virtual key limit reached".into(),
        ));
    }
    let raw = auth::random_token(&account.api_key_prefix);
    let prefix = raw.chars().take(14).collect::<String>();
    let hash = auth::hash_token(&raw);
    let ciphertext = state.crypto.encrypt(&raw).map_err(AppError::Internal)?;
    let mut tx = state.db.begin().await?;
    if let Some(group_id) = input.group_id {
        key_groups::lock_group(&mut tx, user.account_id, group_id).await?;
    }
    let row = sqlx::query("INSERT INTO virtual_api_keys(account_id,name,key_prefix,key_hash,allowed_models,requests_per_minute,token_limit_daily,cost_limit_daily_usd,concurrency_limit,secret_ciphertext,model_mode,cost_limit_total_usd,group_id,holder) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14) RETURNING id")
        .bind(user.account_id).bind(input.name.trim()).bind(prefix).bind(hash).bind(json!(allowed_models)).bind(input.requests_per_minute).bind(input.token_limit_daily).bind(input.cost_limit_daily_usd).bind(input.concurrency_limit.unwrap_or(4).clamp(1, 100)).bind(ciphertext).bind(model_mode).bind(input.cost_limit_total_usd).bind(input.group_id).bind(holder).fetch_one(&mut *tx).await?;
    let id: Uuid =
        sqlx::Row::try_get(&row, "id").map_err(|_| AppError::Internal("invalid key row".into()))?;
    replace_key_channels(&mut tx, id, channel_mode, &channel_ids).await?;
    if let Some(group_id) = input.group_id {
        key_groups::copy_scope(&mut tx, group_id, Some(id)).await?;
    }
    if input.cost_limit_total_usd.is_some() {
        key_quota::record(
            &mut tx,
            id,
            user.account_id,
            user.user_id,
            key_quota::QuotaChange::set(None, input.cost_limit_total_usd),
        )
        .await?;
    }
    tx.commit().await?;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "virtual_key.created",
        "virtual_key",
        id,
        json!({}),
    )
    .await;
    Ok(Json(json!({"id":id,"key":raw})))
}

async fn reveal_key(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response> {
    let user = auth::current_user(&state, &headers).await?;
    let raw = reveal_key_secret(&state, &user, id, json!({})).await?;
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(json!({"key":raw})),
    )
        .into_response())
}

/// Decrypts a key for its own account and records who revealed it.
async fn reveal_key_secret(
    state: &AppState,
    user: &auth::CurrentUser,
    id: Uuid,
    metadata: Value,
) -> Result<String> {
    let ciphertext = sqlx::query_scalar::<_, Option<String>>(
        "SELECT secret_ciphertext FROM virtual_api_keys
         WHERE id=$1 AND account_id=$2 AND revoked_at IS NULL",
    )
    .bind(id)
    .bind(user.account_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?
    .ok_or_else(|| {
        AppError::Conflict("此 Key 创建时仅保存了哈希，无法恢复完整值，请创建新的 API Key。".into())
    })?;
    let raw = state
        .crypto
        .decrypt(&ciphertext)
        .map_err(AppError::Internal)?;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "virtual_key.revealed",
        "virtual_key",
        id,
        metadata,
    )
    .await;
    Ok(raw)
}

async fn revoke_key(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let user = auth::current_user(&state, &headers).await?;
    let mut tx = state.db.begin().await?;
    let result = sqlx::query("UPDATE virtual_api_keys SET enabled=false, revoked_at=now(), secret_ciphertext=NULL WHERE id=$1 AND account_id=$2 AND revoked_at IS NULL").bind(id).bind(user.account_id).execute(&mut *tx).await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    sqlx::query("UPDATE key_share_links SET cancelled_at=now() WHERE virtual_key_id=$1 AND accepted_at IS NULL AND cancelled_at IS NULL")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    state.forget_semaphore(id).await;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "virtual_key.revoked",
        "virtual_key",
        id,
        json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn update_key(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<UpdateVirtualKeyInput>,
) -> Result<StatusCode> {
    let user = auth::current_user(&state, &headers).await?;
    let (current_mode, current_group): (String, Option<Uuid>) = sqlx::query_as("SELECT model_mode,group_id FROM virtual_api_keys WHERE id=$1 AND account_id=$2 AND revoked_at IS NULL")
        .bind(id).bind(user.account_id).fetch_optional(&state.db).await?.ok_or(AppError::NotFound)?;
    // A grouped key ignores the submitted scope and takes the group's.
    let group_id = input.group_id.unwrap_or(current_group);
    let allowed_models = if group_id.is_some() {
        Vec::new()
    } else {
        normalize_allowed_models(input.allowed_models)?
    };
    let model_mode = if group_id.is_some() {
        "all"
    } else {
        input.model_mode.as_deref().unwrap_or(&current_mode)
    };
    model_rules::validate_mode(model_mode, "all", &allowed_models)?;
    let allowed_models = model_rules::key_rules(model_mode, allowed_models);
    if input.name.trim().is_empty() || input.name.len() > 120 {
        return Err(AppError::BadRequest("key name is required".into()));
    }
    let holder = key_holder_name(input.holder.as_ref().and_then(|holder| holder.as_deref()))?;
    if input.requests_per_minute.is_some_and(|value| value <= 0)
        || input.token_limit_daily.is_some_and(|value| value <= 0)
        || input
            .cost_limit_daily_usd
            .is_some_and(|value| !value.is_finite() || value <= 0.0)
        || input
            .cost_limit_total_usd
            .flatten()
            .is_some_and(|value| !value.is_finite() || value <= 0.0)
    {
        return Err(AppError::BadRequest("limits must be positive".into()));
    }
    if !(1..=100).contains(&input.concurrency_limit) {
        return Err(AppError::BadRequest(
            "concurrency limit must be between 1 and 100".into(),
        ));
    }
    let channels = match input.channel_mode.as_deref().filter(|_| group_id.is_none()) {
        Some(mode) => Some((
            mode,
            key_channels(&state.db, user.account_id, mode, input.channel_ids).await?,
        )),
        None => None,
    };
    let mut tx = state.db.begin().await?;
    let limit_before: Option<Option<f64>> = sqlx::query_scalar(
        "SELECT cost_limit_total_usd FROM virtual_api_keys WHERE id=$1 AND account_id=$2 AND revoked_at IS NULL FOR UPDATE",
    )
    .bind(id)
    .bind(user.account_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(limit_before) = limit_before else {
        return Err(AppError::NotFound);
    };
    if let Some(group_id) = group_id {
        key_groups::lock_group(&mut tx, user.account_id, group_id).await?;
    }
    let result = sqlx::query(
        "UPDATE virtual_api_keys SET name=$1, allowed_models=$2, requests_per_minute=$3, token_limit_daily=$4, cost_limit_daily_usd=$5, concurrency_limit=$6, enabled=COALESCE($7,enabled),model_mode=$10,cost_limit_total_usd=CASE WHEN $11 THEN $12 ELSE cost_limit_total_usd END,group_id=$13,holder=CASE WHEN $14 THEN $15 ELSE holder END WHERE id=$8 AND account_id=$9 AND revoked_at IS NULL",
    )
    .bind(input.name.trim())
    .bind(json!(allowed_models))
    .bind(input.requests_per_minute)
    .bind(input.token_limit_daily)
    .bind(input.cost_limit_daily_usd)
    .bind(input.concurrency_limit)
    .bind(input.enabled)
    .bind(id)
    .bind(user.account_id)
    .bind(model_mode)
    .bind(input.cost_limit_total_usd.is_some())
    .bind(input.cost_limit_total_usd.flatten())
    .bind(group_id)
    .bind(input.holder.is_some())
    .bind(holder)
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    if let Some(group_id) = group_id {
        key_groups::copy_scope(&mut tx, group_id, Some(id)).await?;
    } else if let Some((mode, ids)) = channels {
        replace_key_channels(&mut tx, id, mode, &ids).await?;
    }
    if let Some(limit_after) = input.cost_limit_total_usd
        && !key_quota::same_limit(limit_before, limit_after)
    {
        key_quota::record(
            &mut tx,
            id,
            user.account_id,
            user.user_id,
            key_quota::QuotaChange::set(limit_before, limit_after),
        )
        .await?;
    }
    tx.commit().await?;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "virtual_key.updated",
        "virtual_key",
        id,
        json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct KeyEnabledInput {
    enabled: bool,
}

/// Changes only the switch, so a stale key list cannot overwrite other settings.
async fn set_key_enabled(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<KeyEnabledInput>,
) -> Result<StatusCode> {
    let user = auth::current_user(&state, &headers).await?;
    let result = sqlx::query(
        "UPDATE virtual_api_keys SET enabled=$1 WHERE id=$2 AND account_id=$3 AND revoked_at IS NULL",
    )
    .bind(input.enabled)
    .bind(id)
    .bind(user.account_id)
    .execute(&state.db)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "virtual_key.updated",
        "virtual_key",
        id,
        json!({"enabled": input.enabled}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum UsageResetScope {
    Today,
    All,
}

#[derive(Debug, Deserialize)]
struct UsageResetInput {
    scope: UsageResetScope,
}

/// Moves the start of the key's limit windows; usage events are kept for analysis.
async fn reset_key_usage(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<UsageResetInput>,
) -> Result<StatusCode> {
    let user = auth::current_user(&state, &headers).await?;
    let (sql, scope) = match input.scope {
        UsageResetScope::Today => (
            "UPDATE virtual_api_keys SET usage_reset_daily_at=now() WHERE id=$1 AND account_id=$2 AND revoked_at IS NULL",
            "today",
        ),
        UsageResetScope::All => (
            "UPDATE virtual_api_keys SET usage_reset_daily_at=now(),usage_reset_total_at=now() WHERE id=$1 AND account_id=$2 AND revoked_at IS NULL",
            "all",
        ),
    };
    let mut tx = state.db.begin().await?;
    // Lock first, so the usage counted here is the usage the reset clears.
    let limit: Option<Option<f64>> = sqlx::query_scalar(
        "SELECT cost_limit_total_usd FROM virtual_api_keys WHERE id=$1 AND account_id=$2 AND revoked_at IS NULL FOR UPDATE",
    )
    .bind(id)
    .bind(user.account_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(limit) = limit else {
        return Err(AppError::NotFound);
    };
    if scope == "all" {
        let used: f64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(e.cost_usd),0)::float8 FROM virtual_api_keys v
             JOIN usage_events e ON e.account_id=v.account_id AND e.virtual_key_id=v.id
               AND e.created_at>=COALESCE(v.usage_reset_total_at,'-infinity'::timestamptz)
             WHERE v.id=$1",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        key_quota::record(
            &mut tx,
            id,
            user.account_id,
            user.user_id,
            key_quota::QuotaChange {
                kind: "reset",
                amount_usd: None,
                limit_before_usd: limit,
                limit_after_usd: limit,
                used_before_usd: Some(used),
                note: None,
            },
        )
        .await?;
    }
    sqlx::query(sql)
        .bind(id)
        .bind(user.account_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    audit(
        &state.db,
        user.account_id,
        user.user_id,
        "virtual_key.usage_reset",
        "virtual_key",
        id,
        json!({"scope": scope}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn audit_events(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<AuditView>>> {
    let user = auth::current_user(&state, &headers).await?;
    Ok(Json(sqlx::query_as::<_, AuditView>("SELECT id,action,resource_type,resource_id,metadata,created_at FROM audit_logs WHERE account_id=$1 ORDER BY created_at DESC LIMIT 200").bind(user.account_id).fetch_all(&state.db).await?))
}

async fn get_account(db: &PgPool, id: Uuid) -> Result<AccountView> {
    sqlx::query_as::<_, AccountView>(
        "SELECT id,name,api_key_prefix,alert_emails FROM accounts WHERE id=$1",
    )
    .bind(id)
    .fetch_optional(db)
    .await?
    .ok_or(AppError::NotFound)
}

async fn audit(
    db: &PgPool,
    account_id: Uuid,
    user_id: Uuid,
    action: &str,
    resource_type: &str,
    resource_id: Uuid,
    metadata: Value,
) {
    let _ = sqlx::query("INSERT INTO audit_logs(account_id,user_id,action,resource_type,resource_id,metadata) VALUES ($1,$2,$3,$4,$5,$6)")
        .bind(account_id).bind(user_id).bind(action).bind(resource_type).bind(resource_id).bind(metadata).execute(db).await;
}

fn validate_protocol(value: &str) -> Result<()> {
    if matches!(value, "openai" | "anthropic" | "gemini") {
        Ok(())
    } else {
        Err(AppError::BadRequest(
            "protocol must be openai, anthropic, or gemini".into(),
        ))
    }
}

fn normalize_model_mapping(value: Value) -> Result<Value> {
    let object = value
        .as_object()
        .ok_or_else(|| AppError::BadRequest("model_mapping must be an object".into()))?;
    if object.len() > 1000 {
        return Err(AppError::BadRequest("too many model mappings".into()));
    }
    let mut mapping = serde_json::Map::new();
    for (public, upstream) in object {
        let public = public.trim();
        let upstream = upstream.as_str().unwrap_or_default().trim();
        if !model_rules::concrete(public)
            || !model_rules::concrete(upstream)
            || public.len() > 256
            || upstream.len() > 256
            || public.chars().any(char::is_control)
            || upstream.chars().any(char::is_control)
        {
            return Err(AppError::BadRequest(
                "model mappings require valid model names".into(),
            ));
        }
        if mapping
            .insert(public.into(), Value::String(upstream.into()))
            .is_some()
        {
            return Err(AppError::BadRequest(
                "duplicate public model in mapping".into(),
            ));
        }
    }
    Ok(Value::Object(mapping))
}

fn validate_base_url(value: &str) -> Result<()> {
    if value.len() > 2048 {
        return Err(AppError::BadRequest("base URL is too long".into()));
    }
    let parsed =
        Url::parse(value).map_err(|_| AppError::BadRequest("base URL is invalid".into()))?;
    if parsed.scheme() != "https" && parsed.scheme() != "http" {
        return Err(AppError::BadRequest(
            "base URL must use http or https".into(),
        ));
    }
    if parsed.username() != "" || parsed.password().is_some() || parsed.fragment().is_some() {
        return Err(AppError::BadRequest(
            "base URL cannot contain credentials or fragments".into(),
        ));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| AppError::BadRequest("base URL must include a host".into()))?;
    if gateway::blocked_host(host, gateway::private_upstreams_allowed()) {
        return Err(AppError::BadRequest(
            "private or local addresses are not allowed".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_private_channel_targets() {
        assert!(validate_base_url("http://127.0.0.1:9000").is_err());
        assert!(validate_base_url("http://localhost:9000").is_err());
        assert!(validate_base_url("http://10.0.0.8").is_err());
        assert!(validate_base_url("http://[::ffff:127.0.0.1]:9000").is_err());
        assert!(validate_base_url("http://[fd00::1]").is_err());
        assert!(validate_base_url("http://[fe80::1]").is_err());
        assert!(validate_base_url("http://2130706433").is_err());
    }

    #[test]
    fn accepts_public_https_channel_target() {
        assert!(validate_base_url("https://api.example.com/v1").is_ok());
    }

    #[test]
    fn normalizes_model_mapping() {
        let mapping = normalize_model_mapping(json!({" team-chat ": " gpt-4.1 "})).unwrap();
        assert_eq!(mapping["team-chat"], "gpt-4.1");
        assert!(normalize_model_mapping(json!({"team-chat": ""})).is_err());
    }

    #[test]
    fn normalizes_channel_model_prefixes() {
        assert_eq!(
            normalize_channel_models(vec![" gpt-* ".into(), "gpt-4.1".into(), "gpt-*".into()])
                .unwrap(),
            vec!["gpt-*", "gpt-4.1"]
        );
        assert!(normalize_channel_models(vec!["*".into()]).is_err());
        assert!(normalize_channel_models(vec!["gpt*suffix".into()]).is_err());
    }

    #[test]
    fn validates_api_key_prefixes() {
        assert_eq!(
            normalize_api_key_prefix(" team-prod_1 ").unwrap(),
            "team-prod_1"
        );
        assert!(normalize_api_key_prefix("").is_err());
        assert!(normalize_api_key_prefix("-team").is_err());
        assert!(normalize_api_key_prefix("team.prefix").is_err());
        assert!(normalize_api_key_prefix(&"a".repeat(33)).is_err());
    }
}
