//! Passive metering: real gateway traffic shows whether a channel over-reports
//! input tokens or adds a hidden system prompt, without sending any probe.
//! Every request records how many ASCII and other characters of text it sent;
//! each hour a robust regression of input tokens on those counts runs per
//! channel and upstream model. Results only warn, never change routing.
use crate::edition::PRODUCT;
use crate::{auth, email, error::Result, quality_monitors, state::AppState};
use axum::{Json, extract::State, http::HeaderMap};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::Row;
use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use uuid::Uuid;

/// Fewer requests than this in the window give no verdict.
pub const MIN_SAMPLES: usize = 30;
const MAX_SAMPLES: usize = 2000;
/// Conservative limits: English text and code cost about 0.2–0.4 tokens per
/// ASCII character and CJK text about 0.6–1.5 per character.
const MAX_TOKENS_PER_ASCII: f64 = 0.75;
const MAX_TOKENS_PER_OTHER: f64 = 2.5;
/// Chat templates and tool-use instructions add a few hundred tokens; more
/// than this per request suggests a hidden system prompt.
const MAX_OVERHEAD: f64 = 1500.0;
/// A clearly negative overhead means the linear model does not describe the
/// traffic (for example growing agent sessions whose later turns tokenize
/// more densely), so its slopes are not trustworthy either.
const MIN_OVERHEAD: f64 = -500.0;
/// A slope above a limit only counts as inflation when at least this share of
/// requests also exceeds what the limits allow, so a fit skewed by correlated
/// character counts cannot accuse a channel whose totals are all plausible.
const MIN_OVER_LIMIT_SHARE: f64 = 0.2;
/// The overhead is the fit extrapolated to an empty request, so traffic made
/// only of long requests cannot measure it: a slope error of 0.01 tokens per
/// character moves it by a thousand tokens at 100k characters. A hidden prompt
/// is only reported when enough requests shorter than this also carry it.
const SHORT_CHARS: f64 = 8000.0;
const MIN_SHORT_SAMPLES: usize = 5;
const RUN_EVERY: Duration = Duration::from_secs(3600);

/// Keys whose strings are not prompt text.
const SKIP_KEYS: &[&str] = &[
    "model",
    "role",
    "type",
    "id",
    "tool_call_id",
    "tool_use_id",
    "stream",
    "stream_options",
    "cache_control",
    "metadata",
    "user",
    "service_tier",
    "reasoning_effort",
    "stop",
    "stop_sequences",
    "media_type",
    "signature",
];
/// Definitions sent as JSON cost tokens for their structure too.
const SERIALIZED_KEYS: &[&str] = &[
    "tools",
    "functions",
    "functionDeclarations",
    "response_format",
];
/// Content whose token cost is not proportional to characters.
const MEDIA_TYPES: &[&str] = &[
    "image",
    "image_url",
    "input_image",
    "input_audio",
    "input_file",
    "file",
    "document",
    "audio",
    "redacted_thinking",
];
const MEDIA_KEYS: &[&str] = &[
    "image_url",
    "inlineData",
    "inline_data",
    "fileData",
    "file_data",
    "input_audio",
    "encrypted_content",
];

#[derive(Default)]
struct Counts {
    ascii: u64,
    other: u64,
}

impl Counts {
    fn add(&mut self, text: &str) {
        for c in text.chars() {
            if c.is_ascii() {
                self.ascii += 1;
            } else {
                self.other += 1;
            }
        }
    }
}

fn walk(value: &Value, key: Option<&str>, counts: &mut Counts) -> bool {
    match value {
        Value::String(text) => {
            if !key.is_some_and(|k| SKIP_KEYS.contains(&k)) {
                counts.add(text);
            }
            true
        }
        Value::Array(items) => items.iter().all(|item| walk(item, key, counts)),
        Value::Object(map) => {
            if map
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|t| MEDIA_TYPES.contains(&t))
            {
                return false;
            }
            for (k, v) in map {
                if MEDIA_KEYS.contains(&k.as_str()) {
                    return false;
                }
                if SERIALIZED_KEYS.contains(&k.as_str()) {
                    counts.add(&v.to_string());
                } else if !SKIP_KEYS.contains(&k.as_str()) && !walk(v, Some(k), counts) {
                    return false;
                }
            }
            true
        }
        _ => true,
    }
}

/// (ASCII, other) characters of text in a request, or None when it carries
/// images, files or other content whose tokens do not follow characters.
pub fn count_input_chars(payload: &Value) -> Option<(i32, i32)> {
    let mut counts = Counts::default();
    walk(payload, None, &mut counts).then(|| {
        (
            counts.ascii.min(i32::MAX as u64) as i32,
            counts.other.min(i32::MAX as u64) as i32,
        )
    })
}

/// Whether a streamed chunk carries generated output (text, reasoning or a
/// tool call), as opposed to headers, role announcements or empty deltas.
pub fn has_output(chunk: &[u8]) -> bool {
    const MARKERS: &[&[u8]] = &[
        b"content_block_delta",
        b"\"tool_calls\":[",
        b"\"functionCall\"",
    ];
    if MARKERS
        .iter()
        .any(|m| chunk.windows(m.len()).any(|w| w == *m))
    {
        return true;
    }
    // Every Responses API `response.*.delta` event streams generated output
    // (text, reasoning, function or custom tool input such as Codex's
    // apply_patch), so new item types need no marker of their own.
    let event = b"\"type\":\"response.";
    let mut from = 0;
    while let Some(at) = chunk[from..]
        .windows(event.len())
        .position(|w| w == event)
        .map(|i| from + i + event.len())
    {
        let name = chunk[at..].split(|b| *b == b'"').next().unwrap_or_default();
        if name.ends_with(b".delta") {
            return true;
        }
        from = at;
    }
    for key in [
        &b"\"content\""[..],
        b"\"reasoning_content\"",
        b"\"reasoning\"",
        b"\"text\"",
    ] {
        let mut from = 0;
        while let Some(at) = chunk[from..]
            .windows(key.len())
            .position(|w| w == key)
            .map(|i| from + i + key.len())
        {
            let rest = chunk[at..].trim_ascii_start();
            if let Some(rest) = rest.strip_prefix(b":") {
                let rest = rest.trim_ascii_start();
                if rest.starts_with(b"\"") && rest.get(1).is_some_and(|b| *b != b'"') {
                    return true;
                }
            }
            from = at;
        }
    }
    false
}

/// Network chunks need not end at an SSE/JSON boundary. Keep a small suffix
/// so a split output marker still records the first output, without rescanning
/// the full (potentially very long) response on every chunk.
#[derive(Default)]
pub struct OutputDetector {
    tail: Vec<u8>,
}

impl OutputDetector {
    pub fn push(&mut self, chunk: &[u8]) -> bool {
        let mut boundary = std::mem::take(&mut self.tail);
        boundary.extend_from_slice(&chunk[..chunk.len().min(256)]);
        if has_output(&boundary) || has_output(chunk) {
            return true;
        }
        if chunk.len() >= 256 {
            self.tail.extend_from_slice(&chunk[chunk.len() - 256..]);
        } else {
            self.tail
                .extend_from_slice(&boundary[boundary.len().saturating_sub(256)..]);
        }
        false
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Point {
    pub ascii: f64,
    pub other: f64,
    pub tokens: f64,
}

#[derive(Debug, PartialEq)]
pub struct Fit {
    pub per_ascii: Option<f64>,
    pub per_other: Option<f64>,
    pub overhead: f64,
    pub samples: usize,
    /// Share of requests whose input tokens exceed the limits for their ASCII
    /// and other characters plus the overhead limit.
    pub over_limit: f64,
    /// Median tokens of short requests beyond what the slopes explain, or
    /// None when there are too few short requests to measure the overhead.
    pub short_overhead: Option<f64>,
}

/// Solves the weighted normal equations; None when they are singular.
fn solve(rows: &[(Vec<f64>, f64)], weights: &[f64]) -> Option<Vec<f64>> {
    let n = rows.first()?.0.len();
    let mut a = vec![vec![0.0; n + 1]; n];
    for ((x, y), w) in rows.iter().zip(weights) {
        for i in 0..n {
            for j in 0..n {
                a[i][j] += w * x[i] * x[j];
            }
            a[i][n] += w * x[i] * y;
        }
    }
    for col in 0..n {
        let pivot = (col..n).max_by(|&p, &q| a[p][col].abs().total_cmp(&a[q][col].abs()))?;
        if a[pivot][col].abs() < 1e-9 {
            return None;
        }
        a.swap(col, pivot);
        let pivot_row = a[col].clone();
        for (row, values) in a.iter_mut().enumerate() {
            if row != col {
                let factor = values[col] / pivot_row[col];
                for (value, pivot) in values.iter_mut().zip(&pivot_row).skip(col) {
                    *value -= factor * pivot;
                }
            }
        }
    }
    Some((0..n).map(|i| a[i][n] / a[i][i]).collect())
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    }
}

/// Requests with a local token count needed before exact statistics are kept.
const MIN_EXACT_SAMPLES: usize = 30;

/// How reported input tokens compare with local counts, from (expected,
/// reported) pairs. A hidden prompt adds a constant excess; inflation adds
/// one that grows with the request, which shows as a slope.
pub fn exact_stats(pairs: &[(f64, f64)]) -> Option<Value> {
    let pairs: Vec<(f64, f64)> = pairs.iter().copied().filter(|(e, _)| *e > 0.0).collect();
    if pairs.len() < MIN_EXACT_SAMPLES {
        return None;
    }
    let ratios: Vec<f64> = pairs.iter().map(|(e, r)| r / e).collect();
    let excess: Vec<f64> = pairs.iter().map(|(e, r)| r - e).collect();
    let n = pairs.len() as f64;
    let mean_e = pairs.iter().map(|p| p.0).sum::<f64>() / n;
    let mean_x = excess.iter().sum::<f64>() / n;
    let spread: f64 = pairs.iter().map(|p| (p.0 - mean_e).powi(2)).sum();
    let slope = (spread > 0.0).then(|| {
        pairs
            .iter()
            .zip(&excess)
            .map(|(p, x)| (p.0 - mean_e) * (x - mean_x))
            .sum::<f64>()
            / spread
    });
    let round = |v: f64, places: i32| (v * 10f64.powi(places)).round() / 10f64.powi(places);
    Some(json!({
        "samples": pairs.len(),
        "median_ratio": round(median(ratios.clone()), 4),
        "over_1_15": round(ratios.iter().filter(|r| **r > 1.15).count() as f64 / n, 3),
        "median_excess": median(excess).round(),
        "excess_slope": slope.map(|s| round(s, 4)),
        "calibrated": crate::token_estimate::FRAMES_CALIBRATED,
    }))
}

/// Huber regression of input tokens on ASCII and other characters plus a
/// fixed overhead. A character class is modelled only when at least a tenth
/// of the requests use it, and the requests must differ enough in length.
pub fn fit(points: &[Point]) -> Option<Fit> {
    let points: Vec<&Point> = points.iter().filter(|p| p.ascii + p.other > 0.0).collect();
    if points.len() < MIN_SAMPLES {
        return None;
    }
    let lengths: Vec<f64> = points.iter().map(|p| p.ascii + p.other).collect();
    let (shortest, longest) = lengths
        .iter()
        .fold((f64::MAX, 0.0_f64), |(lo, hi), &l| (lo.min(l), hi.max(l)));
    if longest < shortest * 3.0 || longest - shortest < 500.0 {
        return None;
    }
    let used = |pick: fn(&Point) -> f64| {
        points.iter().filter(|p| pick(p) >= 50.0).count() * 10 >= points.len()
    };
    let (use_ascii, use_other) = (used(|p| p.ascii), used(|p| p.other));
    if !use_ascii && !use_other {
        return None;
    }
    let rows: Vec<(Vec<f64>, f64)> = points
        .iter()
        .map(|p| {
            let mut x = Vec::new();
            if use_ascii {
                x.push(p.ascii);
            }
            if use_other {
                x.push(p.other);
            }
            x.push(1.0);
            (x, p.tokens)
        })
        .collect();
    let mut weights = vec![1.0; rows.len()];
    let mut beta = solve(&rows, &weights)?;
    for _ in 0..10 {
        let residuals: Vec<f64> = rows
            .iter()
            .map(|(x, y)| y - x.iter().zip(&beta).map(|(a, b)| a * b).sum::<f64>())
            .collect();
        let scale = (1.4826 * median(residuals.iter().map(|r| r.abs()).collect())).max(1.0);
        let k = 1.345 * scale;
        weights = residuals
            .iter()
            .map(|r| if r.abs() <= k { 1.0 } else { k / r.abs() })
            .collect();
        beta = solve(&rows, &weights)?;
    }
    let mut coefficients = beta.into_iter();
    let per_ascii = use_ascii.then(|| coefficients.next()).flatten();
    let per_other = use_other.then(|| coefficients.next()).flatten();
    let overhead = coefficients.next()?;
    // A slope near zero means the counts do not explain the tokens (for
    // example heavy prompt caching that hides input); no verdict then.
    if per_ascii.into_iter().chain(per_other).any(|s| s < 0.02) || overhead < MIN_OVERHEAD {
        return None;
    }
    let over_limit = points
        .iter()
        .filter(|p| {
            p.tokens
                > p.ascii * MAX_TOKENS_PER_ASCII + p.other * MAX_TOKENS_PER_OTHER + MAX_OVERHEAD
        })
        .count() as f64
        / points.len() as f64;
    // A class too rare to model is charged at its limit, so its characters
    // cannot pass for overhead.
    let short: Vec<f64> = points
        .iter()
        .filter(|p| p.ascii + p.other < SHORT_CHARS)
        .map(|p| {
            p.tokens
                - p.ascii * per_ascii.unwrap_or(MAX_TOKENS_PER_ASCII)
                - p.other * per_other.unwrap_or(MAX_TOKENS_PER_OTHER)
        })
        .collect();
    let short_overhead = (short.len() >= MIN_SHORT_SAMPLES).then(|| median(short));
    Some(Fit {
        per_ascii,
        per_other,
        overhead,
        samples: points.len(),
        over_limit,
        short_overhead,
    })
}

/// Warnings for a fit, each with a stable kind for alert de-duplication.
pub fn judge(fit: &Fit) -> Vec<Value> {
    let round = |v: f64| (v * 1000.0).round() / 1000.0;
    let mut warnings = Vec::new();
    // A slope verdict needs requests that are actually over the limits.
    let over_limit = fit.over_limit >= MIN_OVER_LIMIT_SHARE;
    if let Some(value) = fit
        .per_ascii
        .filter(|v| over_limit && *v > MAX_TOKENS_PER_ASCII)
    {
        warnings.push(json!({"kind":"inflated_ascii","tokens_per_char":round(value),"limit":MAX_TOKENS_PER_ASCII}));
    }
    if let Some(value) = fit
        .per_other
        .filter(|v| over_limit && *v > MAX_TOKENS_PER_OTHER)
    {
        warnings.push(json!({"kind":"inflated_other","tokens_per_char":round(value),"limit":MAX_TOKENS_PER_OTHER}));
    }
    if fit.overhead > MAX_OVERHEAD && fit.short_overhead.is_some_and(|v| v > MAX_OVERHEAD) {
        warnings.push(
            json!({"kind":"hidden_prompt","overhead":fit.overhead.round(),"limit":MAX_OVERHEAD}),
        );
    }
    warnings
}

fn describe(warning: &Value) -> String {
    match warning["kind"].as_str() {
        Some("inflated_ascii") => format!(
            "英文等 ASCII 文本每字符约 {} Token，超过合理上限 {}，疑似用量虚报",
            warning["tokens_per_char"], MAX_TOKENS_PER_ASCII
        ),
        Some("inflated_other") => format!(
            "中文等非 ASCII 文本每字符约 {} Token，超过合理上限 {}，疑似用量虚报",
            warning["tokens_per_char"], MAX_TOKENS_PER_OTHER
        ),
        Some("hidden_prompt") => format!(
            "每个请求固定多出约 {} Token，疑似被加入隐藏系统提示词",
            warning["overhead"]
        ),
        _ => "计量异常".into(),
    }
}

static LAST_RUN: Mutex<Option<Instant>> = Mutex::new(None);
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Starts an hourly pass in the background when one is due.
pub fn run_if_due(state: &AppState) {
    {
        let mut last = LAST_RUN.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|at| at.elapsed() < RUN_EVERY) {
            return;
        }
        if RUNNING.swap(true, Ordering::SeqCst) {
            return;
        }
        *last = Some(Instant::now());
    }
    let state = state.clone();
    tokio::spawn(async move {
        if let Err(error) = run(&state).await {
            eprintln!("passive metering failed: {error}");
        }
        RUNNING.store(false, Ordering::SeqCst);
    });
}

type Group = (Uuid, Vec<Point>, Vec<(f64, f64)>);

/// Fits every channel and upstream model with enough recent traffic, stores
/// the results, alerts on new warnings and runs a service check early.
pub async fn run(state: &AppState) -> Result<()> {
    let run_at: DateTime<Utc> = sqlx::query_scalar("SELECT now()")
        .fetch_one(&state.db)
        .await?;
    let rows = sqlx::query(
        "SELECT account_id,channel_id,COALESCE(upstream_model,model) AS upstream_model,
           input_ascii_chars,COALESCE(input_other_chars,0) AS input_other_chars,input_tokens,input_shape
         FROM usage_events
         WHERE created_at>now()-interval '24 hours' AND input_ascii_chars IS NOT NULL
           AND input_tokens>0 AND status BETWEEN 200 AND 299 AND channel_id IS NOT NULL
         ORDER BY created_at DESC LIMIT 200000",
    )
    .fetch_all(&state.db)
    .await?;
    let mut groups: HashMap<(Uuid, String), Group> = HashMap::new();
    for row in rows {
        let group = groups
            .entry((row.get("channel_id"), row.get("upstream_model")))
            .or_insert_with(|| (row.get("account_id"), Vec::new(), Vec::new()));
        if group.1.len() < MAX_SAMPLES {
            let tokens = row.get::<i64, _>("input_tokens") as f64;
            group.1.push(Point {
                ascii: f64::from(row.get::<i32, _>("input_ascii_chars")),
                other: f64::from(row.get::<i32, _>("input_other_chars")),
                tokens,
            });
            let shape: Option<Value> = row.get("input_shape");
            let model: &str = row.get("upstream_model");
            if let Some(expected) = shape
                .as_ref()
                .and_then(|s| crate::token_estimate::expected_tokens(s, model))
            {
                group.2.push((expected, tokens));
            }
        }
    }
    for ((channel_id, upstream_model), (account_id, points, pairs)) in groups {
        let Some(fit) = fit(&points) else {
            continue;
        };
        // Recorded alongside the regression; warnings stay with the
        // regression until the exact counts are checked on live traffic.
        let exact = exact_stats(&pairs);
        let mut warnings = judge(&fit);
        // A recent service check that measured the same thing overrides the
        // estimate from traffic.
        let check = latest_check(state, channel_id, &upstream_model).await?;
        for warning in &mut warnings {
            let kind = alert_kind(warning);
            if check.as_ref().is_some_and(|c| c.recent && c.refutes(kind)) {
                warning["refuted"] = json!(true);
            }
        }
        let kinds: Vec<&str> = warnings
            .iter()
            .filter(|w| w["refuted"] != true)
            .map(alert_kind)
            .collect();
        sqlx::query(
            "DELETE FROM metering_alerts WHERE channel_id=$1 AND upstream_model=$2 AND source='passive'
               AND NOT kind=ANY($3)",
        )
        .bind(channel_id)
        .bind(&upstream_model)
        .bind(&kinds)
        .execute(&state.db)
        .await?;
        let alerted: Vec<String> = sqlx::query_scalar(
            "SELECT kind FROM metering_alerts WHERE channel_id=$1 AND upstream_model=$2",
        )
        .bind(channel_id)
        .bind(&upstream_model)
        .fetch_all(&state.db)
        .await?;
        sqlx::query(
            "INSERT INTO channel_passive_stats(channel_id,upstream_model,account_id,samples,tokens_per_ascii,
               tokens_per_other,overhead,warnings,alerted,computed_at,exact)
             SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11 WHERE EXISTS(SELECT 1 FROM channels WHERE id=$1 AND account_id=$3)
             ON CONFLICT (channel_id,upstream_model) DO UPDATE SET samples=$4,tokens_per_ascii=$5,tokens_per_other=$6,
               overhead=$7,warnings=$8,alerted=$9,computed_at=$10,exact=$11",
        )
        .bind(channel_id)
        .bind(&upstream_model)
        .bind(account_id)
        .bind(fit.samples as i32)
        .bind(fit.per_ascii)
        .bind(fit.per_other)
        .bind(fit.overhead)
        .bind(json!(warnings))
        .bind(json!(kinds))
        .bind(run_at)
        .bind(exact)
        .execute(&state.db)
        .await?;
        let new: Vec<&Value> = warnings
            .iter()
            .filter(|w| w["refuted"] != true && !alerted.iter().any(|a| a == alert_kind(w)))
            .collect();
        if new.is_empty() {
            continue;
        }
        let monitors = monitors_for(state, account_id, channel_id, &upstream_model).await?;
        // A service check confirms before anyone is emailed when it can
        // measure the warning: any check sees a hidden prompt, only a local
        // count sees proportional inflation.
        let emailed: Vec<&Value> = new
            .into_iter()
            .filter(|w| {
                monitors.is_empty()
                    || !(alert_kind(w) == "hidden_prompt"
                        || check.as_ref().is_some_and(|c| c.exact))
            })
            .collect();
        for id in &monitors {
            // A daily limit or a job already running only skips the early check.
            let _ = quality_monitors::enqueue(&state.db, account_id, *id, "triggered").await;
        }
        if emailed.is_empty() {
            continue;
        }
        for warning in &emailed {
            sqlx::query(
                "INSERT INTO metering_alerts(channel_id,upstream_model,kind,source) VALUES($1,$2,$3,'passive')
                 ON CONFLICT DO NOTHING",
            )
            .bind(channel_id)
            .bind(&upstream_model)
            .bind(alert_kind(warning))
            .execute(&state.db)
            .await?;
        }
        notify(
            state,
            account_id,
            channel_id,
            &upstream_model,
            &fit,
            &emailed,
            !monitors.is_empty(),
        )
        .await?;
    }
    sqlx::query("DELETE FROM channel_passive_stats WHERE computed_at<$1")
        .bind(run_at)
        .execute(&state.db)
        .await?;
    Ok(())
}

/// The warnings passive metering and service checks share for alerting.
fn alert_kind(warning: &Value) -> &'static str {
    match warning["kind"].as_str() {
        Some("hidden_prompt") => "hidden_prompt",
        _ => "inflated",
    }
}

/// The latest metering result of a service check on the channel and model.
struct Check {
    exact: bool,
    hidden_prompt: bool,
    inflated: bool,
    /// Within the window of the traffic it is compared with.
    recent: bool,
}

impl Check {
    fn refutes(&self, kind: &str) -> bool {
        match kind {
            "hidden_prompt" => !self.hidden_prompt,
            _ => self.exact && !self.inflated,
        }
    }
}

async fn latest_check(
    state: &AppState,
    channel_id: Uuid,
    upstream_model: &str,
) -> Result<Option<Check>> {
    let row = sqlx::query(
        "SELECT t->'detail' AS detail, r.finished_at>now()-interval '24 hours' AS recent
         FROM channel_monitor_runs r CROSS JOIN LATERAL jsonb_array_elements(r.details->'tests') t
         WHERE r.channel_id=$1 AND r.kind='quality' AND r.details->>'upstream_model'=$2
           AND r.details->>'applied'='true' AND t->>'id'='metering' AND t->'detail' ? 'method'
         ORDER BY r.finished_at DESC LIMIT 1",
    )
    .bind(channel_id)
    .bind(upstream_model)
    .fetch_optional(&state.db)
    .await?;
    Ok(row.map(|r| {
        let detail: Value = r.get("detail");
        Check {
            exact: detail["method"] == "exact",
            hidden_prompt: detail["hidden_prompt"] == true,
            inflated: detail["inflated"] == true,
            recent: r.get("recent"),
        }
    }))
}

async fn monitors_for(
    state: &AppState,
    account_id: Uuid,
    channel_id: Uuid,
    upstream_model: &str,
) -> Result<Vec<Uuid>> {
    Ok(sqlx::query_scalar(
        "SELECT q.id FROM quality_monitors q JOIN channels c ON c.id=q.channel_id
         WHERE q.channel_id=$1 AND c.account_id=$2 AND q.kind='service' AND q.deleted_at IS NULL AND c.enabled
           AND COALESCE(c.model_mapping->>q.model,q.model)=$3",
    )
    .bind(channel_id)
    .bind(account_id)
    .bind(upstream_model)
    .fetch_all(&state.db)
    .await?)
}

async fn notify(
    state: &AppState,
    account_id: Uuid,
    channel_id: Uuid,
    upstream_model: &str,
    fit: &Fit,
    warnings: &[&Value],
    checked: bool,
) -> Result<()> {
    let channel: Option<String> = sqlx::query_scalar("SELECT name FROM channels WHERE id=$1")
        .bind(channel_id)
        .fetch_optional(&state.db)
        .await?;
    let lines: Vec<String> = warnings
        .iter()
        .map(|w| format!("- {}", describe(w)))
        .collect();
    email::account_alert(
        state,
        account_id,
        email::AlertKind::Metering,
        "计量异常告警",
        format!(
            "渠道「{}」的模型 {upstream_model} 在最近 24 小时的真实请求中（{} 个）出现计量异常：\n{}\n\n{}请登录 {PRODUCT} 在「渠道监控 → 服务体检」中查看。这是根据真实流量估算的结果，不影响路由。",
            channel.unwrap_or_default(),
            fit.samples,
            lines.join("\n"),
            if checked {
                "服务体检无法按本地计数核对这项，已提前运行一次体检作参考。\n"
            } else {
                "该模型还没有服务体检，添加后可以用主动计量检查确认。\n"
            },
        ),
    )
    .await
}

/// Emails a metering problem a service check confirmed, once per kind, and
/// clears the alerts a passing check covers. An exact count confirms with
/// one failure; the estimate needs `previous_failed` too.
pub async fn settle_check(
    state: &AppState,
    account_id: Uuid,
    channel_id: Uuid,
    model: &str,
    upstream_model: &str,
    detail: &Value,
    previous_failed: bool,
) -> Result<()> {
    let exact = detail["method"] == "exact";
    let covered: &[&str] = if exact {
        &["hidden_prompt", "inflated"]
    } else {
        &["hidden_prompt"]
    };
    let failed: Vec<&str> = [
        ("hidden_prompt", detail["hidden_prompt"] == true),
        ("inflated", detail["inflated"] == true),
    ]
    .into_iter()
    .filter_map(|(kind, flagged)| flagged.then_some(kind))
    .collect();
    let confirmed = !failed.is_empty() && (exact || previous_failed);
    let cleared: Vec<&str> = covered
        .iter()
        .copied()
        .filter(|k| !failed.contains(k))
        .collect();
    sqlx::query(
        "DELETE FROM metering_alerts WHERE channel_id=$1 AND upstream_model=$2 AND kind=ANY($3)",
    )
    .bind(channel_id)
    .bind(upstream_model)
    .bind(&cleared)
    .execute(&state.db)
    .await?;
    if !confirmed {
        return Ok(());
    }
    let new: Vec<String> = sqlx::query_scalar(
        "INSERT INTO metering_alerts(channel_id,upstream_model,kind,source)
         SELECT $1,$2,k,'check' FROM unnest($3::text[]) k ON CONFLICT DO NOTHING RETURNING kind",
    )
    .bind(channel_id)
    .bind(upstream_model)
    .bind(&failed)
    .fetch_all(&state.db)
    .await?;
    if new.is_empty() {
        return Ok(());
    }
    let channel: String = sqlx::query_scalar("SELECT name FROM channels WHERE id=$1")
        .bind(channel_id)
        .fetch_one(&state.db)
        .await?;
    let lines: Vec<String> = new
        .iter()
        .map(|kind| format!("- {}", describe_check(kind, detail)))
        .collect();
    email::account_alert(
        state,
        account_id,
        email::AlertKind::Metering,
        "计量异常告警",
        format!(
            "渠道「{channel}」的模型 {model} 的服务体检确认了计量异常：\n{}\n\n{}请登录 {PRODUCT} 在「渠道监控 → 服务体检」中查看。计量检查只告警，不影响路由。",
            lines.join("\n"),
            if exact {
                "这是用本地 Token 计数逐个请求比对的结果。\n"
            } else {
                "这是连续两次体检估算的结果。\n"
            },
        ),
    )
    .await
}

fn describe_check(kind: &str, detail: &Value) -> String {
    match (kind, detail["method"] == "exact") {
        ("hidden_prompt", _) => format!(
            "每个请求多出约 {} Token，疑似被加入隐藏系统提示词",
            detail["fixed_overhead"]
        ),
        (_, true) => format!(
            "上报的输入 Token 为本地计数的 {} 倍，疑似用量虚报",
            detail["ratio"]
        ),
        _ => format!(
            "每字符约 {} Token，超过合理上限，疑似用量虚报",
            detail["tokens_per_char"]
        ),
    }
}

/// Passive metering results of the account, with the service check (if any)
/// each one belongs to.
pub async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let user = auth::current_user(&state, &headers).await?;
    let rows = sqlx::query(
        "SELECT s.channel_id,c.name AS channel_name,s.upstream_model,s.samples,s.tokens_per_ascii,s.tokens_per_other,
           s.overhead,s.warnings,s.computed_at,s.exact,
           (SELECT q.id FROM quality_monitors q WHERE q.channel_id=s.channel_id AND q.kind='service' AND q.deleted_at IS NULL
              AND COALESCE(c.model_mapping->>q.model,q.model)=s.upstream_model ORDER BY q.created_at LIMIT 1) AS monitor_id
         FROM channel_passive_stats s JOIN channels c ON c.id=s.channel_id AND c.account_id=s.account_id
         WHERE s.account_id=$1 ORDER BY c.name,s.upstream_model",
    )
    .bind(user.account_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!(
        rows.iter()
            .map(|r| {
                // Warnings a service check refuted are kept for the record only.
                let warnings: Vec<Value> = r
                    .get::<Value, _>("warnings")
                    .as_array()
                    .map(|w| w.iter().filter(|w| w["refuted"] != true).cloned().collect())
                    .unwrap_or_default();
                let described: Vec<String> = warnings.iter().map(describe).collect();
                json!({"channel_id":r.get::<Uuid, _>("channel_id"),"channel_name":r.get::<String, _>("channel_name"),
                    "upstream_model":r.get::<String, _>("upstream_model"),"samples":r.get::<i32, _>("samples"),
                    "tokens_per_ascii":r.get::<Option<f64>, _>("tokens_per_ascii"),
                    "tokens_per_other":r.get::<Option<f64>, _>("tokens_per_other"),
                    "overhead":r.get::<f64, _>("overhead"),"warnings":warnings,"messages":described,
                    "computed_at":r.get::<DateTime<Utc>, _>("computed_at"),
                    "exact":r.get::<Option<Value>, _>("exact"),
                    "monitor_id":r.get::<Option<Uuid>, _>("monitor_id")})
            })
            .collect::<Vec<_>>()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{Rng, SeedableRng, rngs::StdRng};

    #[test]
    fn output_detector_handles_every_chunk_boundary() {
        let output = b"data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\n";
        for split in 1..output.len() {
            let mut detector = OutputDetector::default();
            assert!(!detector.push(
                b"data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"\"}}]}\n\n"
            ));
            assert!(
                detector.push(&output[..split]) || detector.push(&output[split..]),
                "split {split}"
            );
        }
        let mut detector = OutputDetector::default();
        for _ in 0..1000 {
            assert!(!detector.push(b": keep-alive\n\n"));
            assert!(detector.tail.len() <= 256);
        }
    }

    #[test]
    fn exact_stats_separate_hidden_prompts_from_inflation() {
        assert!(exact_stats(&[(1000.0, 1000.0); 29]).is_none());
        let sizes = (0..40).map(|i| 1000.0 + 2000.0 * f64::from(i));
        let hidden: Vec<_> = sizes.clone().map(|e| (e, e + 800.0)).collect();
        let stats = exact_stats(&hidden).unwrap();
        assert_eq!(stats["median_excess"], 800.0);
        assert_eq!(stats["excess_slope"], 0.0);
        let inflated: Vec<_> = sizes.map(|e| (e, e * 1.3)).collect();
        let stats = exact_stats(&inflated).unwrap();
        assert_eq!(stats["median_ratio"], 1.3);
        assert_eq!(stats["over_1_15"], 1.0);
        assert_eq!(stats["excess_slope"], 0.3);
    }

    #[test]
    fn counts_text_and_rejects_media() {
        let openai = json!({"model":"gpt","messages":[{"role":"system","content":"You are 助手"},
            {"role":"user","content":[{"type":"text","text":"hi"}]}],"stream":true});
        assert_eq!(count_input_chars(&openai), Some((10, 2)));
        let tools = json!({"messages":[{"role":"user","content":"a"}],"tools":[{"name":"f"}]});
        assert_eq!(count_input_chars(&tools), Some((1 + 14, 0)));
        let image = json!({"messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"data:"}}]}]});
        assert_eq!(count_input_chars(&image), None);
        let gemini = json!({"contents":[{"parts":[{"text":"x"},{"inlineData":{"data":"AAA"}}]}]});
        assert_eq!(count_input_chars(&gemini), None);
    }

    #[test]
    fn output_is_detected_only_in_generated_deltas() {
        assert!(!has_output(
            br#"data: {"choices":[{"delta":{"role":"assistant","content":""}}]}"#
        ));
        assert!(has_output(
            br#"data: {"choices":[{"delta":{"content":"Hi"}}]}"#
        ));
        assert!(!has_output(
            br#"event: message_start
data: {"type":"message_start","message":{"content":[]}}"#
        ));
        assert!(has_output(
            br#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"x"}}"#
        ));
        assert!(has_output(
            b"[{\n  \"candidates\": [{\"content\": {\"parts\": [{\"text\": \"Hel\"}]}}]"
        ));
    }

    #[test]
    fn responses_deltas_count_as_output_for_every_item_type() {
        for event in [
            "output_text",
            "reasoning_summary_text",
            "reasoning_text",
            "function_call_arguments",
            "custom_tool_call_input",
        ] {
            let chunk = format!(
                "event: response.{event}.delta\ndata: {{\"type\":\"response.{event}.delta\",\"item_id\":\"ctc_1\",\"output_index\":1,\"delta\":\"*** Begin Patch\"}}\n\n"
            );
            assert!(has_output(chunk.as_bytes()), "{event}");
        }
        for chunk in [
            br#"data: {"type":"response.created","response":{"id":"resp_1","status":"in_progress"}}"#
                .as_slice(),
            br#"data: {"type":"response.output_item.added","item":{"type":"custom_tool_call","id":"ctc_1","name":"apply_patch","input":""}}"#,
            br#"data: {"type":"response.custom_tool_call_input.done","item_id":"ctc_1","input":""}"#,
        ] {
            assert!(!has_output(chunk));
        }
    }

    fn traffic(per_ascii: f64, per_other: f64, overhead: f64, seed: u64) -> Vec<Point> {
        let mut rng = StdRng::seed_from_u64(seed);
        (0..200)
            .map(|i| {
                let ascii = rng.gen_range(200.0..40000.0);
                let other = if i % 3 == 0 {
                    rng.gen_range(100.0..5000.0)
                } else {
                    0.0
                };
                let noise = rng.gen_range(-0.05..0.05);
                let mut tokens = (ascii * per_ascii + other * per_other) * (1.0 + noise) + overhead;
                // A few requests with cached or odd usage must not move the fit.
                if i % 50 == 0 {
                    tokens *= 0.2;
                }
                Point {
                    ascii,
                    other,
                    tokens,
                }
            })
            .collect()
    }

    #[test]
    fn honest_traffic_passes_and_tampering_is_flagged() {
        let honest = fit(&traffic(0.28, 0.9, 120.0, 1)).unwrap();
        assert!(
            (honest.per_ascii.unwrap() - 0.28).abs() < 0.02,
            "{honest:?}"
        );
        assert!((honest.per_other.unwrap() - 0.9).abs() < 0.1, "{honest:?}");
        assert!(judge(&honest).is_empty());
        let hidden = fit(&traffic(0.28, 0.9, 3500.0, 2)).unwrap();
        assert_eq!(judge(&hidden)[0]["kind"], "hidden_prompt");
        let inflated = fit(&traffic(1.1, 0.9, 120.0, 3)).unwrap();
        assert_eq!(judge(&inflated)[0]["kind"], "inflated_ascii");
    }

    /// Real counts of a Claude Code channel: a few growing sessions whose
    /// later turns tokenize more densely while Chinese text grows with them,
    /// plus five short requests with a fixed prompt added by the relay.
    const AGENT_SESSIONS: &[(f64, f64, f64)] = &[
        (97533.0, 954.0, 40145.0),
        (100216.0, 958.0, 42122.0),
        (0.0, 10.0, 2612.0),
        (0.0, 10.0, 2612.0),
        (0.0, 10.0, 2612.0),
        (0.0, 10.0, 2612.0),
        (0.0, 10.0, 2612.0),
        (105633.0, 968.0, 44965.0),
        (106557.0, 1261.0, 45743.0),
        (107257.0, 1263.0, 46162.0),
        (116432.0, 1730.0, 51025.0),
        (127096.0, 1730.0, 55012.0),
        (138491.0, 1780.0, 59345.0),
        (144326.0, 1790.0, 62468.0),
        (146695.0, 1875.0, 63854.0),
        (158558.0, 1875.0, 68986.0),
        (169600.0, 1880.0, 73787.0),
        (173426.0, 1936.0, 76030.0),
        (176101.0, 2540.0, 78043.0),
        (96097.0, 264.0, 38702.0),
        (98796.0, 293.0, 40237.0),
        (103270.0, 1277.0, 43958.0),
        (111959.0, 2582.0, 49808.0),
        (116237.0, 5226.0, 55368.0),
        (126935.0, 6392.0, 61846.0),
        (136318.0, 6627.0, 66642.0),
        (147522.0, 6661.0, 72627.0),
        (160016.0, 6883.0, 80287.0),
        (168084.0, 7026.0, 84340.0),
        (173514.0, 7036.0, 87999.0),
        (189073.0, 7066.0, 93311.0),
        (210323.0, 7143.0, 99775.0),
        (252502.0, 9345.0, 127635.0),
        (253560.0, 9403.0, 128288.0),
        (255625.0, 9405.0, 129410.0),
        (77242.0, 222.0, 30336.0),
        (97527.0, 955.0, 39221.0),
        (98317.0, 956.0, 39665.0),
        (100082.0, 956.0, 40654.0),
        (114721.0, 956.0, 46494.0),
        (127854.0, 1006.0, 50803.0),
        (133939.0, 1009.0, 53855.0),
        (135070.0, 1009.0, 54468.0),
        (137324.0, 1348.0, 56137.0),
        (139214.0, 1348.0, 56955.0),
        (141162.0, 1348.0, 57984.0),
        (143977.0, 1348.0, 59428.0),
        (148480.0, 1352.0, 61738.0),
        (151102.0, 1354.0, 63318.0),
        (154211.0, 1384.0, 64959.0),
        (157765.0, 1384.0, 66713.0),
        (159632.0, 1386.0, 67657.0),
        (161656.0, 1387.0, 68699.0),
        (162623.0, 1388.0, 69269.0),
        (163944.0, 2199.0, 70969.0),
    ];

    fn agent_sessions(scale: f64, short_requests: bool) -> Vec<Point> {
        AGENT_SESSIONS
            .iter()
            .filter(|(ascii, ..)| short_requests || *ascii > 0.0)
            .map(|&(ascii, other, tokens)| Point {
                ascii,
                other,
                tokens: tokens * scale,
            })
            .collect()
    }

    #[test]
    fn correlated_agent_sessions_are_not_flagged_unless_totals_exceed_limits() {
        let honest = fit(&agent_sessions(1.0, true)).unwrap();
        // The slope alone is skewed past the limit by correlated counts.
        assert!(
            honest.per_other.unwrap() > MAX_TOKENS_PER_OTHER,
            "{honest:?}"
        );
        assert!(judge(&honest).is_empty(), "{honest:?}");
        // Without the short requests the fit needs a negative overhead.
        assert!(fit(&agent_sessions(1.0, false)).is_none());
        let inflated = fit(&agent_sessions(3.0, true)).unwrap();
        // Tripled usage puts the totals over the limits, so it is still flagged.
        let warnings = judge(&inflated);
        let kinds: Vec<&str> = warnings.iter().filter_map(|w| w["kind"].as_str()).collect();
        assert_eq!(kinds, ["inflated_ascii", "inflated_other"], "{inflated:?}");
    }

    #[test]
    fn hidden_prompt_needs_short_requests_that_carry_it() {
        let hidden: Vec<Point> = traffic(0.28, 0.9, 3500.0, 5);
        let long: Vec<Point> = hidden
            .iter()
            .copied()
            .filter(|p| p.ascii + p.other >= SHORT_CHARS)
            .collect();
        // Long requests alone extrapolate an overhead they cannot confirm.
        let unconfirmed = fit(&long).unwrap();
        assert!(unconfirmed.overhead > MAX_OVERHEAD, "{unconfirmed:?}");
        assert_eq!(unconfirmed.short_overhead, None);
        assert!(judge(&unconfirmed).is_empty());
        // Short requests without the overhead contradict the extrapolation.
        let mut contradicted = long.clone();
        contradicted.extend((0..10).map(|i| Point {
            ascii: 3000.0 + f64::from(i) * 100.0,
            other: 200.0,
            tokens: 0.28 * (3000.0 + f64::from(i) * 100.0) + 0.9 * 200.0 + 100.0,
        }));
        let contradicted = fit(&contradicted).unwrap();
        assert!(
            contradicted.short_overhead.unwrap() < MAX_OVERHEAD,
            "{contradicted:?}"
        );
        assert!(judge(&contradicted).is_empty(), "{contradicted:?}");
        let confirmed = fit(&hidden).unwrap();
        assert!(
            confirmed.short_overhead.unwrap() > MAX_OVERHEAD,
            "{confirmed:?}"
        );
        assert_eq!(judge(&confirmed)[0]["kind"], "hidden_prompt");
    }

    #[test]
    fn too_little_or_too_uniform_traffic_gives_no_verdict() {
        assert!(fit(&traffic(0.3, 0.9, 100.0, 4)[..20]).is_none());
        let uniform: Vec<Point> = (0..100)
            .map(|_| Point {
                ascii: 1000.0,
                other: 0.0,
                tokens: 400.0,
            })
            .collect();
        assert!(fit(&uniform).is_none());
    }
}
