//! Long-context checks: does the channel really read a long input? A random
//! passphrase hidden at a random depth of random text must be found, tier by
//! tier. Inputs reach 128k tokens, so checks run only on demand, with a cost
//! estimate and the same risk acknowledgement as authenticity checks.
use crate::{
    auth,
    error::{AppError, Result},
    pricing, service_checks,
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use rand::{Rng, seq::SliceRandom};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

/// Approximate input tokens of each tier.
pub const TIERS: [i32; 3] = [8_000, 32_000, 128_000];
/// A 128k request can take minutes to read.
pub const REQUEST_TIMEOUT_MS: i32 = 300_000;
/// Long enough for every tier in sequence.
pub const LEASE_SECONDS: i32 = 1_200;

const WORDS: &[&str] = &[
    "river", "window", "garden", "pencil", "orange", "silver", "planet", "button", "candle",
    "forest", "ladder", "marble", "pocket", "rabbit", "saddle", "tunnel", "violet", "wallet",
    "anchor", "basket", "copper", "desert", "engine", "fabric", "harbor", "island", "jacket",
    "kettle", "meadow", "needle", "pepper", "rocket", "shadow", "ticket", "valley", "yellow",
    "bridge", "castle", "falcon", "hammer", "jungle", "lantern", "mirror", "parrot", "ribbon",
    "summit", "timber", "velvet", "willow", "bamboo", "feather", "granite", "pillow", "thunder",
];

pub struct Probe {
    pub prompt: String,
    pub passphrase: String,
}

/// Random words of about `tokens` tokens with a passphrase at a random depth.
pub fn plan(tokens: i32, rng: &mut impl Rng) -> Probe {
    // Common English words cost about 1.3 tokens each with their space.
    let count = (tokens as usize * 3 / 4).max(100);
    let passphrase = format!(
        "{}-{}",
        WORDS.choose(rng).unwrap().to_uppercase(),
        rng.gen_range(1000..10000)
    );
    let depth = rng.gen_range(count / 10..count * 9 / 10);
    let mut text = String::with_capacity(count * 8);
    for i in 0..count {
        if i == depth {
            text.push_str(&format!("The secret passphrase is {passphrase}. "));
        }
        text.push_str(WORDS.choose(rng).unwrap());
        text.push(' ');
    }
    Probe {
        prompt: format!(
            "{text}\n\n上面的文本中藏有一句 \"The secret passphrase is ...\"。请只输出这个口令，不要输出其他内容。"
        ),
        passphrase,
    }
}

pub fn request(probe: &Probe) -> service_checks::Request {
    service_checks::Request {
        tag: "long_context",
        kind: service_checks::Kind::Chat,
        prompt: probe.prompt.clone(),
        stream: false,
        max_tokens: 1024,
    }
}

async fn context(db: &sqlx::PgPool, account_id: Uuid, id: Uuid) -> Result<sqlx::postgres::PgRow> {
    sqlx::query(
        "SELECT q.long_context_max,q.long_context_checked_at,c.enabled,c.authenticity_ack_at IS NOT NULL AS acknowledged,
           COALESCE(c.model_mapping->>q.model,r.upstream_model,q.model) AS upstream_model,
           (SELECT status FROM quality_monitor_jobs WHERE monitor_id=q.id AND status IN ('queued','running') LIMIT 1) AS active
         FROM quality_monitors q JOIN channels c ON c.id=q.channel_id
         LEFT JOIN LATERAL (SELECT upstream_model FROM model_routes WHERE channel_id=c.id AND account_id=$2
             AND public_model=q.model AND protocol=c.protocol AND enabled ORDER BY priority,id LIMIT 1) r ON true
         WHERE q.id=$1 AND c.account_id=$2 AND q.kind='service' AND q.deleted_at IS NULL",
    )
    .bind(id)
    .bind(account_id)
    .fetch_optional(db)
    .await?
    .ok_or(AppError::NotFound)
}

/// Tiers with their estimated cost (every tier up to and including it), and the last result.
pub async fn options(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    let row = context(&state.db, user.account_id, id).await?;
    let upstream: String = row.get("upstream_model");
    let prices = pricing::load(&state.db).await?;
    let price = pricing::lookup(&prices, &upstream);
    let mut cumulative = 0.0;
    let tiers: Vec<Value> = TIERS
        .iter()
        .map(|&tokens| {
            let cost = price.map(|p| {
                cumulative += p.estimate(i64::from(tokens), 20, 0);
                cumulative
            });
            json!({"tokens":tokens,"cost_usd":cost})
        })
        .collect();
    Ok(Json(json!({"tiers":tiers,"upstream_model":upstream,
        "acknowledged":row.get::<bool, _>("acknowledged"),"active":row.get::<Option<String>, _>("active"),
        "max_passed":row.get::<Option<i32>, _>("long_context_max"),
        "checked_at":row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("long_context_checked_at")})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartInput {
    pub max_tokens: i32,
    #[serde(default)]
    pub acknowledge_risk: bool,
}

/// Queues a long-context check up to the chosen tier.
pub async fn enqueue(
    db: &sqlx::PgPool,
    account_id: Uuid,
    user_id: Uuid,
    id: Uuid,
    input: &StartInput,
) -> Result<Uuid> {
    if !TIERS.contains(&input.max_tokens) {
        return Err(AppError::BadRequest("请选择 8k、32k 或 128k 档位".into()));
    }
    let mut tx = db.begin().await?;
    let row = sqlx::query(
        "SELECT q.config_version,c.id AS channel_id,c.enabled,c.authenticity_ack_at IS NOT NULL AS acknowledged
         FROM quality_monitors q JOIN channels c ON c.id=q.channel_id
         WHERE q.id=$1 AND c.account_id=$2 AND q.kind='service' AND q.deleted_at IS NULL FOR UPDATE OF q",
    )
    .bind(id)
    .bind(account_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    if !row.get::<bool, _>("enabled") {
        return Err(AppError::Conflict("渠道已停用，请先启用渠道".into()));
    }
    if !row.get::<bool, _>("acknowledged") {
        if !input.acknowledge_risk {
            return Err(AppError::Refused(
                "authenticity_ack_required",
                "首次检测前需要确认风险提示".into(),
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
    let busy: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM quality_monitor_jobs WHERE monitor_id=$1 AND status IN ('queued','running'))",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    if busy {
        return Err(AppError::Conflict(
            "该监控已有检测在排队或运行，请等待完成".into(),
        ));
    }
    let job: Uuid = sqlx::query_scalar(
        "INSERT INTO quality_monitor_jobs(monitor_id,config_version,source,options) VALUES($1,$2,'long_context',$3) RETURNING id",
    )
    .bind(id)
    .bind(row.get::<i64, _>("config_version"))
    .bind(json!({"max_tokens":input.max_tokens}))
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(job)
}

pub async fn start(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<StartInput>,
) -> Result<(StatusCode, Json<Value>)> {
    let user = auth::current_user(&state, &headers).await?;
    let job = enqueue(&state.db, user.account_id, user.user_id, id, &input).await?;
    crate::audit(
        &state.db,
        user.account_id,
        user.user_id,
        "quality_monitor.long_context_started",
        "quality_monitor",
        id,
        json!({"job_id":job,"max_tokens":input.max_tokens}),
    )
    .await;
    Ok((StatusCode::ACCEPTED, Json(json!({"job_id":job}))))
}

/// The largest tier passed in a row from the smallest, and whether every
/// requested tier finished (an unfinished tier says nothing about the channel).
pub fn summarize(results: &[(i32, Option<bool>)]) -> (Option<i32>, &'static str) {
    let max = results
        .iter()
        .take_while(|(_, passed)| *passed == Some(true))
        .map(|(tier, _)| *tier)
        .last();
    let status = match results.iter().find(|(_, passed)| *passed != Some(true)) {
        None => "healthy",
        Some((_, Some(false))) => "degraded",
        Some(_) => "unknown",
    };
    (max, status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{SeedableRng, rngs::StdRng};

    #[test]
    fn passphrase_is_hidden_inside_the_text() {
        let probe = plan(8_000, &mut StdRng::seed_from_u64(7));
        let at = probe.prompt.find(&probe.passphrase).unwrap();
        assert!(at > probe.prompt.len() / 20 && at < probe.prompt.len() * 19 / 20);
        assert_eq!(probe.prompt.matches(&probe.passphrase).count(), 1);
        assert_ne!(
            plan(8_000, &mut StdRng::seed_from_u64(8)).passphrase,
            probe.passphrase
        );
    }

    #[test]
    fn summary_stops_at_the_first_failure() {
        assert_eq!(
            summarize(&[(8_000, Some(true)), (32_000, Some(true))]),
            (Some(32_000), "healthy")
        );
        assert_eq!(
            summarize(&[(8_000, Some(true)), (32_000, Some(false))]),
            (Some(8_000), "degraded")
        );
        assert_eq!(summarize(&[(8_000, None)]), (None, "unknown"));
    }
}
