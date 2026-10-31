use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::{
    auth::hash_token,
    crypto::Crypto,
    error::{AppError, Result},
};

pub fn test_model<'a>(
    explicit: Option<&'a str>,
    rules: &'a [String],
    mapping: &'a serde_json::Value,
) -> Option<&'a str> {
    explicit.or_else(|| {
        // Older channels may store rules unsorted; the default is the first by name.
        rules
            .iter()
            .filter(|model| crate::model_rules::concrete(model))
            .min()
            .map(|model| {
                mapping
                    .get(model)
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(model)
            })
    })
}

#[derive(Deserialize, Serialize)]
pub struct SuccessfulCheck {
    fingerprint: String,
    pub model: String,
    pub checked_at: DateTime<Utc>,
    pub ttfb_ms: Option<i32>,
    /// The upstream answered 429: reachable and accepting the key, but busy.
    #[serde(default)]
    pub rate_limited: bool,
}

pub struct CheckConfig<'a> {
    pub account_id: Uuid,
    pub channel_id: Option<Uuid>,
    pub protocol: &'a str,
    pub base_url: &'a str,
    pub secret: &'a str,
    pub model: &'a str,
}

impl CheckConfig<'_> {
    fn fingerprint(&self) -> String {
        hash_token(
            &json!([
                "channel-preview-v1",
                self.account_id,
                self.channel_id,
                self.protocol,
                self.base_url.trim(),
                self.secret,
                self.model,
            ])
            .to_string(),
        )
    }

    pub fn receipt(
        &self,
        crypto: &Crypto,
        ttfb_ms: Option<i32>,
        rate_limited: bool,
    ) -> Result<String> {
        let check = SuccessfulCheck {
            fingerprint: self.fingerprint(),
            model: self.model.to_owned(),
            checked_at: Utc::now(),
            ttfb_ms,
            rate_limited,
        };
        let value = serde_json::to_string(&check)
            .map_err(|_| AppError::Internal("check serialization failed".into()))?;
        crypto.encrypt(&value).map_err(AppError::Internal)
    }

    // Only a recent server-issued result for this account and exact configuration
    // can initialize health; a client-provided healthy flag is never trusted.
    pub fn verify(&self, crypto: &Crypto, receipt: &str) -> Result<SuccessfulCheck> {
        let invalid = || AppError::BadRequest("连接测试已失效，请重新测试。".into());
        let value = crypto.decrypt(receipt).map_err(|_| invalid())?;
        let check: SuccessfulCheck = serde_json::from_str(&value).map_err(|_| invalid())?;
        let age = Utc::now() - check.checked_at;
        if check.fingerprint != self.fingerprint()
            || age < Duration::zero()
            || age > Duration::minutes(15)
        {
            return Err(invalid());
        }
        Ok(check)
    }
}

pub async fn save_health(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    channel_id: Uuid,
    check: &SuccessfulCheck,
) -> Result<()> {
    let (status, message) = if check.rate_limited {
        ("rate_limited", "上游限流（HTTP 429），本次不计入失败")
    } else {
        ("healthy", "检查通过")
    };
    // A rate-limited check still reached the upstream with a valid key.
    let saved = sqlx::query("INSERT INTO channel_health(channel_id,status,last_observation,last_error,checked_at) VALUES ($1,'healthy',$3,$4,$2) ON CONFLICT(channel_id) DO UPDATE SET status='healthy',last_observation=$3,last_error=$4,check_note=NULL,last_error_kind=NULL,consecutive_failures=0,failing_since=NULL,last_failure_at=NULL,checked_at=$2,updated_at=now() WHERE channel_health.checked_at IS NULL OR channel_health.checked_at < EXCLUDED.checked_at")
        .bind(channel_id).bind(check.checked_at).bind(check.rate_limited.then_some("rate_limited")).bind(check.rate_limited.then_some(message)).execute(&mut **tx).await?;
    if saved.rows_affected() == 0 {
        return Ok(());
    }
    sqlx::query("INSERT INTO channel_monitor_runs(channel_id,kind,model,status,ttfb_ms,details,started_at,finished_at) VALUES ($1,'availability',$2,$5,$3,$6,$4,$4)")
        .bind(channel_id).bind(&check.model).bind(check.ttfb_ms).bind(check.checked_at).bind(status).bind(json!({"message": message})).execute(&mut **tx).await?;
    sqlx::query("UPDATE channel_monitor_configs SET next_availability_at=$2 + make_interval(secs => availability_interval_seconds),updated_at=now() WHERE channel_id=$1")
        .bind(channel_id).bind(check.checked_at).execute(&mut **tx).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_test_model_is_the_first_concrete_rule_by_name() {
        let rules = ["zeta".to_owned(), "demo-*".to_owned(), "alpha".to_owned()];
        let mapping = json!({"alpha": "alpha-upstream"});
        assert_eq!(test_model(None, &rules, &mapping), Some("alpha-upstream"));
        assert_eq!(test_model(Some("chosen"), &rules, &mapping), Some("chosen"));
        assert_eq!(test_model(None, &rules[1..2], &mapping), None);
    }
}
