//! Cooldowns for rate-limited channels and the routing settings that tune them.
//! See docs/design/routing-upstream-errors.md.

use crate::{
    auth,
    error::{AppError, Result},
    state::AppState,
};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, header},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RoutingSettings {
    /// A channel that returns a rate limit is tried after the others for a while.
    pub cooldown_enabled: bool,
    /// Cooldowns double on repeated rate limits up to this.
    pub max_cooldown_seconds: u64,
    /// The upstream's `Retry-After` replaces the doubling schedule.
    pub honor_retry_after: bool,
}

impl Default for RoutingSettings {
    fn default() -> Self {
        Self {
            cooldown_enabled: true,
            max_cooldown_seconds: 300,
            honor_retry_after: true,
        }
    }
}

/// The first cooldown after a rate limit, before doubling.
const FIRST_COOLDOWN_SECONDS: u64 = 30;
/// Shortest cooldown taken from `Retry-After`.
const MIN_RETRY_AFTER_SECONDS: u64 = 5;
const MAX_COOLDOWN_RANGE: std::ops::RangeInclusive<u64> = 30..=3600;
/// How often the gateway rereads the settings, for changes saved by another instance.
const REFRESH_INTERVAL: Duration = Duration::from_secs(30);
/// A slow Redis must not hold up routing; cooldowns are skipped instead.
const READ_TIMEOUT: Duration = Duration::from_millis(200);

pub async fn load(db: &PgPool) -> RoutingSettings {
    let row: Option<String> = sqlx::query_scalar(
        "SELECT value_ciphertext FROM system_settings WHERE key='routing_settings'",
    )
    .fetch_optional(db)
    .await
    .ok()
    .flatten();
    row.and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

/// Keeps the cached settings current.
pub fn start_refresh(state: AppState) {
    tokio::spawn(async move {
        loop {
            let settings = load(&state.db).await;
            set_cached(&state, settings);
            tokio::time::sleep(REFRESH_INTERVAL).await;
        }
    });
}

fn set_cached(state: &AppState, settings: RoutingSettings) {
    *state
        .routing_settings
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = settings;
}

pub async fn get_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<RoutingSettings>> {
    auth::require_super_admin(&state, &headers).await?;
    Ok(Json(load(&state.db).await))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateRoutingSettingsInput {
    cooldown_enabled: Option<bool>,
    max_cooldown_seconds: Option<u64>,
    honor_retry_after: Option<bool>,
}

pub async fn update_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<UpdateRoutingSettingsInput>,
) -> Result<Json<RoutingSettings>> {
    let user = auth::require_super_admin(&state, &headers).await?;
    let current = load(&state.db).await;
    let updated = RoutingSettings {
        cooldown_enabled: input.cooldown_enabled.unwrap_or(current.cooldown_enabled),
        max_cooldown_seconds: input
            .max_cooldown_seconds
            .unwrap_or(current.max_cooldown_seconds),
        honor_retry_after: input.honor_retry_after.unwrap_or(current.honor_retry_after),
    };
    if !MAX_COOLDOWN_RANGE.contains(&updated.max_cooldown_seconds) {
        return Err(AppError::BadRequest(format!(
            "最长冷却时长需在 {} 到 {} 秒之间",
            MAX_COOLDOWN_RANGE.start(),
            MAX_COOLDOWN_RANGE.end()
        )));
    }
    let json = serde_json::to_string(&updated)
        .map_err(|_| AppError::Internal("routing settings serialization failed".into()))?;
    sqlx::query(
        "INSERT INTO system_settings(key,value_ciphertext) VALUES ('routing_settings',$1)
         ON CONFLICT (key) DO UPDATE SET value_ciphertext=EXCLUDED.value_ciphertext,updated_at=now()",
    )
    .bind(&json)
    .execute(&state.db)
    .await?;
    set_cached(&state, updated);
    crate::audit(
        &state.db,
        user.account_id,
        user.user_id,
        "platform.routing_settings_updated",
        "system_settings",
        user.user_id,
        json!(updated),
    )
    .await;
    Ok(Json(updated))
}

/// A channel model's rate-limit state when the request was routed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cooldown {
    /// Rate limited recently: tried after the channels that are not.
    pub cooling: bool,
    /// Rate limits in a row are being counted; a success clears them.
    pub strikes: bool,
}

fn cooldown_key(channel_id: Uuid, model: &str) -> String {
    format!("meta-router:cooldown:{channel_id}:{model}")
}

fn strikes_key(channel_id: Uuid, model: &str) -> String {
    format!("meta-router:cooldown_strikes:{channel_id}:{model}")
}

/// Cooldown state for each channel model, in order. Without Redis, or with
/// cooldowns turned off, nothing is cooling.
pub async fn cooldowns(state: &AppState, targets: &[(Uuid, &str)]) -> Vec<Cooldown> {
    let none = vec![Cooldown::default(); targets.len()];
    if targets.is_empty() || !state.routing_settings().cooldown_enabled {
        return none;
    }
    let keys: Vec<String> = targets
        .iter()
        .flat_map(|&(channel_id, model)| {
            [
                cooldown_key(channel_id, model),
                strikes_key(channel_id, model),
            ]
        })
        .collect();
    let read = async {
        let mut conn = state.redis_connection().await?;
        redis::cmd("MGET")
            .arg(&keys)
            .query_async::<Vec<Option<String>>>(&mut conn)
            .await
    };
    match tokio::time::timeout(READ_TIMEOUT, read).await {
        Ok(Ok(values)) if values.len() == keys.len() => values
            .chunks(2)
            .map(|pair| Cooldown {
                cooling: pair[0].is_some(),
                strikes: pair[1].is_some(),
            })
            .collect(),
        _ => none,
    }
}

/// Starts or extends a channel model's cooldown after a rate limit, in the background.
pub fn start_cooldown(state: &AppState, channel_id: Uuid, model: &str, retry_after: Option<u64>) {
    let settings = state.routing_settings();
    if !settings.cooldown_enabled {
        return;
    }
    let state = state.clone();
    let (cooldown, strikes) = (
        cooldown_key(channel_id, model),
        strikes_key(channel_id, model),
    );
    tokio::spawn(async move {
        let Ok(mut conn) = state.redis_connection().await else {
            return;
        };
        let counted: redis::RedisResult<(i64, i64)> = redis::pipe()
            .atomic()
            .incr(&strikes, 1)
            .expire(&strikes, (settings.max_cooldown_seconds * 2) as i64)
            .query_async(&mut conn)
            .await;
        let Ok((count, _)) = counted else {
            return;
        };
        let seconds = cooldown_seconds(settings, count, retry_after);
        let _: redis::RedisResult<()> = redis::cmd("SET")
            .arg(&cooldown)
            .arg(1)
            .arg("EX")
            .arg(seconds)
            .query_async(&mut conn)
            .await;
    });
}

/// Restarts the doubling after a channel model answers again, in the background.
pub fn clear_strikes(state: &AppState, channel_id: Uuid, model: &str) {
    let state = state.clone();
    let strikes = strikes_key(channel_id, model);
    tokio::spawn(async move {
        if let Ok(mut conn) = state.redis_connection().await {
            let _: redis::RedisResult<i64> =
                redis::cmd("DEL").arg(&strikes).query_async(&mut conn).await;
        }
    });
}

/// `Retry-After` if honored, bounded; otherwise 30 seconds doubling per
/// rate limit in a row, up to the maximum.
fn cooldown_seconds(settings: RoutingSettings, strikes: i64, retry_after: Option<u64>) -> u64 {
    let max = settings.max_cooldown_seconds;
    if settings.honor_retry_after
        && let Some(seconds) = retry_after
    {
        return seconds.clamp(MIN_RETRY_AFTER_SECONDS, max);
    }
    let doublings = strikes.clamp(1, 32) as u32 - 1;
    FIRST_COOLDOWN_SECONDS
        .saturating_mul(1u64 << doublings.min(20))
        .min(max)
}

/// Seconds to wait from a `Retry-After` header, either form.
pub fn retry_after(headers: &HeaderMap) -> Option<u64> {
    let value = headers.get(header::RETRY_AFTER)?.to_str().ok()?.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(seconds);
    }
    let at = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    let seconds = (at.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_seconds();
    Some(seconds.max(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooldown_doubles_up_to_the_maximum() {
        let settings = RoutingSettings::default();
        let seconds: Vec<u64> = (1..=6)
            .map(|strikes| cooldown_seconds(settings, strikes, None))
            .collect();
        assert_eq!(seconds, [30, 60, 120, 240, 300, 300]);
        assert_eq!(cooldown_seconds(settings, 1000, None), 300);
        let longer = RoutingSettings {
            max_cooldown_seconds: 3600,
            ..settings
        };
        assert_eq!(cooldown_seconds(longer, 8, None), 3600);
    }

    #[test]
    fn retry_after_is_bounded_and_can_be_ignored() {
        let settings = RoutingSettings::default();
        assert_eq!(cooldown_seconds(settings, 3, Some(1)), 5);
        assert_eq!(cooldown_seconds(settings, 3, Some(42)), 42);
        assert_eq!(cooldown_seconds(settings, 1, Some(10_000)), 300);
        let ignored = RoutingSettings {
            honor_retry_after: false,
            ..settings
        };
        assert_eq!(cooldown_seconds(ignored, 2, Some(5)), 60);
    }

    #[test]
    fn retry_after_reads_seconds_and_dates() {
        let mut headers = HeaderMap::new();
        assert_eq!(retry_after(&headers), None);
        headers.insert(header::RETRY_AFTER, "17".parse().unwrap());
        assert_eq!(retry_after(&headers), Some(17));
        let at = chrono::Utc::now() + chrono::Duration::seconds(90);
        headers.insert(
            header::RETRY_AFTER,
            at.format("%a, %d %b %Y %H:%M:%S GMT")
                .to_string()
                .parse()
                .unwrap(),
        );
        assert!((85..=90).contains(&retry_after(&headers).unwrap()));
        headers.insert(
            header::RETRY_AFTER,
            "Wed, 21 Oct 2015 07:28:00 GMT".parse().unwrap(),
        );
        assert_eq!(retry_after(&headers), Some(0));
        headers.insert(header::RETRY_AFTER, "soon".parse().unwrap());
        assert_eq!(retry_after(&headers), None);
    }

    #[test]
    fn stored_settings_fill_missing_fields() {
        let partial: RoutingSettings =
            serde_json::from_str(r#"{"cooldown_enabled":false}"#).unwrap();
        assert_eq!(
            partial,
            RoutingSettings {
                cooldown_enabled: false,
                ..RoutingSettings::default()
            }
        );
    }
}
