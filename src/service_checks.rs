//! Service checks (quality monitoring suite v6): is the channel delivering the
//! service it claims, untampered? Every round uses fresh random parameters, so
//! answers cannot be cached or matched, and no model baseline is needed.
use rand::{Rng, seq::SliceRandom};
use serde_json::{Value, json};

/// Configurable methods. "integrity" also yields the replay check.
pub const METHODS: [&str; 5] = ["integrity", "metering", "tools", "truncation", "streaming"];
pub const SUITE_VERSION: &str = "v6";
/// Checks whose confirmed failure may affect routing; the rest only warn.
pub const CRITICAL: [&str; 2] = ["integrity", "replay"];
/// Above this many input tokens per character the channel over-reports usage.
/// Random lowercase English words normally cost about 0.2–0.4.
const MAX_TOKENS_PER_CHAR: f64 = 0.75;
/// A fixed overhead beyond the chat template and instruction suggests a
/// hidden system prompt added to every request. Both requests are random
/// words from one list, so an honest fit lands within about 40 tokens.
const MAX_FIXED_OVERHEAD: f64 = 100.0;
/// With a local count, reported input tokens may exceed it by this many
/// tokens or this share, whichever is larger: calibrated counts land within
/// 8 tokens or 1.7%.
const EXACT_TOLERANCE: f64 = 30.0;
const EXACT_SHARE: f64 = 0.05;
/// Extra text beyond the requested answer worth pointing out (ads, notices).
const EXTRA_TEXT_WARNING: usize = 120;
const MAX_TOKENS: u32 = 1024;
/// Room for the numbered list (a few hundred tokens) and some reasoning, so
/// only a cap well below what was asked for cuts the list short.
const TRUNCATION_MAX_TOKENS: u32 = 4096;
const TOOL_NAME: &str = "lookup_order";

/// Metering against local counts of the short and long requests, each an
/// (expected, reported) pair. A hidden prompt shows as excess already on the
/// short request; inflation as excess that grows with the request.
fn exact_metering(short: (f64, f64), long: (f64, f64)) -> Judged {
    let allowed = |expected: f64| (expected * EXACT_SHARE).max(EXACT_TOLERANCE);
    let (short_excess, long_excess) = (short.1 - short.0, long.1 - long.0);
    let hidden = short_excess > allowed(short.0);
    let inflated = long_excess - short_excess.max(0.0) > allowed(long.0);
    Judged {
        id: "metering",
        passed: Some(!hidden && !inflated),
        critical: false,
        detail: json!({"method":"exact","inflation_checked":true,"expected":[short.0, long.0],
            "reported":[short.1, long.1],"excess":[short_excess.round(), long_excess.round()],
            "fixed_overhead":short_excess.round(),"ratio":(long.1 / long.0 * 1000.0).round() / 1000.0,
            "inflated":inflated,"hidden_prompt":hidden}),
        tags: Vec::new(),
        error: None,
    }
}

/// Display name of a check for alerts.
pub fn label(id: &str) -> &'static str {
    match id {
        "integrity" => "回答完整性",
        "replay" => "缓存重放",
        "metering" => "计量检查",
        "tools" => "工具调用",
        "truncation" => "输出截断",
        "streaming" => "流式输出",
        _ => "其他检查",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Chat,
    Tool,
}

#[derive(Clone, Debug)]
pub struct Request {
    /// integrity_1, integrity_2, metering_short, metering_long, tools,
    /// truncation or streaming.
    pub tag: &'static str,
    pub kind: Kind,
    pub prompt: String,
    pub stream: bool,
    pub max_tokens: u32,
}

impl Request {
    fn chat(tag: &'static str, prompt: String) -> Self {
        Self {
            tag,
            kind: Kind::Chat,
            prompt,
            stream: false,
            max_tokens: MAX_TOKENS,
        }
    }
}

#[derive(Debug)]
pub struct Round {
    pub requests: Vec<Request>,
    /// (nonce, expected sum) per integrity request.
    answers: Vec<(String, i64)>,
    order_id: String,
    /// (line count, marker) of the numbered list.
    list: Option<(usize, String)>,
    /// Code the streamed answer must echo.
    echo: Option<String>,
}

const WORDS: &[&str] = &[
    "river", "window", "garden", "pencil", "orange", "silver", "planet", "button", "candle",
    "forest", "ladder", "marble", "pocket", "rabbit", "saddle", "tunnel", "violet", "wallet",
    "anchor", "basket", "copper", "desert", "engine", "fabric", "glider", "harbor", "island",
    "jacket", "kettle", "lemon", "meadow", "needle", "oyster", "pepper", "quartz", "rocket",
    "shadow", "ticket", "uniform", "valley", "walnut", "yellow", "zipper", "bridge", "castle",
    "dragon", "falcon", "ginger", "hammer", "insect", "jungle", "kitten", "lantern", "mirror",
    "napkin", "orchid", "parrot", "quiver", "ribbon", "summit", "timber", "umbrella", "velvet",
    "willow", "almond", "bamboo", "cactus", "dolphin", "eclipse", "feather", "granite", "hollow",
    "iceberg", "jasmine", "kernel", "lizard", "magnet", "nectar", "outlet", "pillow", "quarry",
    "raven", "spiral", "thunder", "upward", "vessel", "whisker", "yogurt", "zenith", "blanket",
];

fn words(rng: &mut impl Rng, count: usize) -> String {
    (0..count)
        .map(|_| *WORDS.choose(rng).unwrap())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Uppercase letters only (no I/O), so digits in the answer are the sum.
fn nonce(rng: &mut impl Rng) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";
    (0..24)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect()
}

/// Four distinct words joined by hyphens, letters only like the nonce. Asked
/// to copy a random letter code, Claude refused about a third of the time; a
/// test phrase of words is refused a few times in a hundred.
fn phrase(rng: &mut impl Rng) -> String {
    WORDS
        .choose_multiple(rng, 4)
        .copied()
        .collect::<Vec<_>>()
        .join("-")
}

fn integrity_prompt(rng: &mut impl Rng, phrase: &str, a: i64, b: i64) -> String {
    match rng.gen_range(0..3) {
        0 => format!(
            "这是一次接口连通性测试。请只回复两行：第一行写本次的测试口令 {phrase}，第二行写 {a} 加 {b} 等于多少，只写数字。"
        ),
        1 => format!(
            "This is an API connectivity test. Reply with two lines only: first the test phrase {phrase}, then the value of {a} + {b} in digits."
        ),
        _ => format!(
            "接口自检：请先写出测试口令 {phrase}，换行后写出 {a}+{b} 的结果。只写这两行，不用解释。"
        ),
    }
}

pub fn plan(methods: &[String], rng: &mut impl Rng) -> Round {
    let mut requests = Vec::new();
    let mut answers = Vec::new();
    if methods.iter().any(|m| m == "integrity") {
        for tag in ["integrity_1", "integrity_2"] {
            let (code, a, b) = (
                phrase(rng),
                rng.gen_range(100..1000),
                rng.gen_range(100..1000),
            );
            requests.push(Request::chat(tag, integrity_prompt(rng, &code, a, b)));
            answers.push((code, a + b));
        }
    }
    if methods.iter().any(|m| m == "metering") {
        for (tag, count) in [("metering_short", 25), ("metering_long", 220)] {
            requests.push(Request::chat(
                tag,
                format!(
                    "下面是一串随机单词，不需要理解，只回复 OK。\n{}",
                    words(rng, count)
                ),
            ));
        }
    }
    let order_id = format!("ORD-{}", rng.gen_range(100_000..1_000_000));
    if methods.iter().any(|m| m == "tools") {
        requests.push(Request {
            kind: Kind::Tool,
            ..Request::chat(
                "tools",
                format!("请调用 {TOOL_NAME} 工具查询订单 {order_id} 的状态，不要直接回答。"),
            )
        });
    }
    let mut list = None;
    if methods.iter().any(|m| m == "truncation") {
        let (lines, marker) = (rng.gen_range(40..=60), nonce(rng)[..6].to_owned());
        requests.push(Request {
            max_tokens: TRUNCATION_MAX_TOKENS,
            ..Request::chat(
                "truncation",
                format!(
                    "请输出 {lines} 行，第 k 行的内容是「k. {marker}」，k 从 1 到 {lines}。只输出这 {lines} 行，不要输出其他内容。"
                ),
            )
        });
        list = Some((lines, marker));
    }
    let mut echo = None;
    if methods.iter().any(|m| m == "streaming") {
        let code = phrase(rng);
        requests.push(Request {
            stream: true,
            ..Request::chat(
                "streaming",
                format!("这是一次接口连通性测试。请只回复本次的测试口令，不要加其他内容：{code}"),
            )
        });
        echo = Some(code);
    }
    Round {
        requests,
        answers,
        order_id,
        list,
        echo,
    }
}

/// Upstream path and body for a request in the channel's protocol.
pub fn body(protocol: &str, model: &str, request: &Request) -> (String, Value) {
    let parameters = json!({"type":"object","properties":{"order_id":{"type":"string","description":"Order id"}},"required":["order_id"]});
    let description = "Look up the status of an order by its id.";
    let tool = request.kind == Kind::Tool;
    match protocol {
        "anthropic" => {
            let mut body = json!({"model":model,"max_tokens":request.max_tokens,"messages":[{"role":"user","content":request.prompt}]});
            if request.stream {
                body["stream"] = json!(true);
            }
            if tool {
                body["tools"] =
                    json!([{"name":TOOL_NAME,"description":description,"input_schema":parameters}]);
            }
            ("/v1/messages".to_owned(), body)
        }
        "gemini" => {
            let mut body = json!({"contents":[{"parts":[{"text":request.prompt}]}],"generationConfig":{"maxOutputTokens":request.max_tokens}});
            if tool {
                body["tools"] = json!([{"functionDeclarations":[{"name":TOOL_NAME,"description":description,"parameters":parameters}]}]);
            }
            let path = if request.stream {
                format!("/v1beta/models/{model}:streamGenerateContent?alt=sse")
            } else {
                format!("/v1beta/models/{model}:generateContent")
            };
            (path, body)
        }
        _ => {
            let mut body = json!({"model":model,"messages":[{"role":"user","content":request.prompt}],
                "max_tokens":request.max_tokens,"stream":request.stream,"reasoning_effort":"low"});
            if request.stream {
                body["stream_options"] = json!({"include_usage":true});
            }
            if tool {
                body["tools"] = json!([{"type":"function","function":{"name":TOOL_NAME,"description":description,"parameters":parameters}}]);
            }
            ("/v1/chat/completions".to_owned(), body)
        }
    }
}

/// The first tool call in a response: (name, arguments).
pub fn tool_call(protocol: &str, payload: &Value) -> Option<(String, Value)> {
    match protocol {
        "anthropic" => payload["content"].as_array()?.iter().find_map(|block| {
            if block["type"] != "tool_use" {
                return None;
            }
            Some((block["name"].as_str()?.to_owned(), block["input"].clone()))
        }),
        "gemini" => payload
            .pointer("/candidates/0/content/parts")?
            .as_array()?
            .iter()
            .find_map(|part| {
                let call = part.get("functionCall")?;
                Some((call["name"].as_str()?.to_owned(), call["args"].clone()))
            }),
        _ => {
            let function = payload.pointer("/choices/0/message/tool_calls/0/function")?;
            let arguments = match &function["arguments"] {
                Value::String(text) => serde_json::from_str(text).unwrap_or(Value::Null),
                other => other.clone(),
            };
            Some((function["name"].as_str()?.to_owned(), arguments))
        }
    }
}

/// Why generation stopped: "stop" for a normal end (including a tool call),
/// "length" at the output cap, anything else as reported.
pub fn finish_reason(protocol: &str, payload: &Value) -> Option<String> {
    let raw = match protocol {
        "anthropic" => payload["stop_reason"].as_str(),
        "gemini" => payload.pointer("/candidates/0/finishReason")?.as_str(),
        _ => payload.pointer("/choices/0/finish_reason")?.as_str(),
    }?;
    Some(normalize_finish(raw))
}

fn normalize_finish(raw: &str) -> String {
    match raw {
        "stop" | "end_turn" | "stop_sequence" | "tool_use" | "tool_calls" | "STOP" => "stop",
        "length" | "max_tokens" | "MAX_TOKENS" => "length",
        "SAFETY" | "PROHIBITED_CONTENT" | "BLOCKLIST" | "SPII" => "content_filter",
        other => other,
    }
    .to_owned()
}

/// Whether a response carried reasoning output, which can use up the output
/// cap before any answer is written.
pub fn reasoning(protocol: &str, payload: &Value) -> bool {
    let filled = |value: Option<&Value>| {
        value
            .and_then(Value::as_str)
            .is_some_and(|t| !t.trim().is_empty())
    };
    match protocol {
        "anthropic" => payload["content"].as_array().is_some_and(|blocks| {
            blocks
                .iter()
                .any(|b| b["type"] == "thinking" || b["type"] == "redacted_thinking")
        }),
        "gemini" => {
            payload
                .pointer("/candidates/0/content/parts")
                .and_then(Value::as_array)
                .is_some_and(|parts| parts.iter().any(|p| p["thought"] == true))
                || payload
                    .pointer("/usageMetadata/thoughtsTokenCount")
                    .and_then(Value::as_i64)
                    .is_some_and(|n| n > 0)
        }
        _ => {
            let message = payload.pointer("/choices/0/message");
            filled(message.and_then(|m| m.get("reasoning_content")))
                || filled(message.and_then(|m| m.get("reasoning")))
                || payload
                    .pointer("/usage/completion_tokens_details/reasoning_tokens")
                    .and_then(Value::as_i64)
                    .is_some_and(|n| n > 0)
        }
    }
}

/// The upstream's safety filter stopped the answer. Claude refuses some
/// harmless check prompts, so this says nothing about the channel.
pub fn blocked(finish: Option<&str>) -> Option<&'static str> {
    match finish? {
        "refusal" => Some("quality_probe_finish_refusal"),
        "content_filter" => Some("quality_probe_finish_content_filter"),
        _ => None,
    }
}

/// Why a reply without an answer cannot be judged: a safety filter, the
/// output cap or reasoning took it. None means the reply is simply blank,
/// which fails.
pub fn unanswered(finish: Option<&str>, reasoning: bool) -> Option<&'static str> {
    if let Some(error) = blocked(finish) {
        Some(error)
    } else if finish == Some("length") {
        Some("quality_probe_truncated")
    } else if reasoning {
        Some("quality_probe_reasoning_only")
    } else {
        None
    }
}

/// How a streamed answer arrived.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamReport {
    /// Every data event parsed as JSON and at least one arrived.
    pub events_ok: bool,
    /// The stream ended the way the protocol ends it.
    pub ended: bool,
    /// Usage was reported inside the stream.
    pub usage_reported: bool,
}

/// A streamed answer: its text, why it stopped and whether it reasoned.
#[derive(Debug, Default)]
pub struct Streamed {
    pub text: Option<String>,
    pub finish: Option<String>,
    pub reasoning: bool,
    pub report: StreamReport,
}

/// The text of a server-sent event stream and how well-formed it was.
pub fn parse_stream(protocol: &str, body: &[u8]) -> Streamed {
    let mut text = String::new();
    let mut finish = None;
    let mut reasoning = false;
    let mut report = StreamReport {
        events_ok: true,
        usage_reported: crate::gateway::parse_stream_usage(body).is_some(),
        ..Default::default()
    };
    let mut events = 0;
    for line in String::from_utf8_lossy(body).lines() {
        let Some(data) = line.trim().strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data == "[DONE]" {
            report.ended = true;
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(data) else {
            report.events_ok = false;
            continue;
        };
        events += 1;
        match protocol {
            "anthropic" => match event["type"].as_str() {
                Some("content_block_delta") => match event
                    .pointer("/delta/type")
                    .and_then(Value::as_str)
                {
                    Some("thinking_delta") => reasoning = true,
                    _ => {
                        if let Some(delta) = event.pointer("/delta/text").and_then(Value::as_str) {
                            text.push_str(delta);
                        }
                    }
                },
                Some("content_block_start") => {
                    if matches!(
                        event.pointer("/content_block/type").and_then(Value::as_str),
                        Some("thinking" | "redacted_thinking")
                    ) {
                        reasoning = true;
                    }
                }
                Some("message_delta") => {
                    if let Some(raw) = event.pointer("/delta/stop_reason").and_then(Value::as_str) {
                        finish = Some(normalize_finish(raw));
                    }
                }
                Some("message_stop") => report.ended = true,
                _ => {}
            },
            "gemini" => {
                for part in event
                    .pointer("/candidates/0/content/parts")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if part["thought"] == true {
                        reasoning = true;
                    } else {
                        text.push_str(part["text"].as_str().unwrap_or(""));
                    }
                }
                if event
                    .pointer("/usageMetadata/thoughtsTokenCount")
                    .and_then(Value::as_i64)
                    .is_some_and(|n| n > 0)
                {
                    reasoning = true;
                }
                if let Some(raw) = event
                    .pointer("/candidates/0/finishReason")
                    .and_then(Value::as_str)
                {
                    finish = Some(normalize_finish(raw));
                    report.ended = true;
                }
            }
            _ => {
                let delta = event.pointer("/choices/0/delta");
                if let Some(content) = delta.and_then(|d| d.get("content")).and_then(Value::as_str)
                {
                    text.push_str(content);
                }
                if ["reasoning_content", "reasoning"].iter().any(|key| {
                    delta
                        .and_then(|d| d.get(*key))
                        .and_then(Value::as_str)
                        .is_some_and(|t| !t.is_empty())
                }) {
                    reasoning = true;
                }
                if let Some(raw) = event
                    .pointer("/choices/0/finish_reason")
                    .and_then(Value::as_str)
                {
                    finish = Some(normalize_finish(raw));
                }
            }
        }
    }
    report.events_ok &= events > 0;
    Streamed {
        text: (!text.trim().is_empty()).then_some(text),
        finish,
        reasoning,
        report,
    }
}

/// What one request returned, as far as judging needs.
#[derive(Debug, Default)]
pub struct Reply {
    pub text: Option<String>,
    pub tool: Option<(String, Value)>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub finish: Option<String>,
    /// The reply carried reasoning output.
    pub reasoning: bool,
    pub stream: Option<StreamReport>,
    /// Input tokens the request should cost by local count, when the model is
    /// calibrated on this API.
    pub expected_input: Option<f64>,
    /// Why the request was not counted locally.
    pub uncounted: Option<&'static str>,
    /// Transport, HTTP or parse error: the check is unfinished, not failed.
    pub error: Option<String>,
}

#[derive(Debug)]
pub struct Judged {
    pub id: &'static str,
    /// None when the check could not finish.
    pub passed: Option<bool>,
    pub critical: bool,
    pub detail: Value,
    /// Requests the check used, for latency and status.
    pub tags: Vec<&'static str>,
    pub error: Option<String>,
}

fn reply_for<'a>(round: &Round, replies: &'a [Reply], tag: &str) -> Option<&'a Reply> {
    round
        .requests
        .iter()
        .position(|r| r.tag == tag)
        .and_then(|i| replies.get(i))
}

/// The text with thousands separators between digits removed, so "1,234" reads as 1234.
fn without_separators(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    chars
        .iter()
        .enumerate()
        .filter(|&(i, &c)| {
            let separator = c == ','
                && i > 0
                && chars[i - 1].is_ascii_digit()
                && chars.len() > i + 3
                && chars[i + 1..=i + 3].iter().all(char::is_ascii_digit)
                && !chars.get(i + 4).is_some_and(char::is_ascii_digit);
            !separator
        })
        .map(|(_, c)| *c)
        .collect()
}

fn integrity_ok(text: &str, (code, sum): &(String, i64)) -> (bool, usize) {
    let ints = crate::authenticity::fingerprint::parse_ints(&without_separators(text));
    let extra = text
        .trim()
        .chars()
        .count()
        .saturating_sub(code.len() + sum.to_string().len() + 1);
    (text.contains(code.as_str()) && ints.contains(sum), extra)
}

pub fn judge(round: &Round, replies: &[Reply]) -> Vec<Judged> {
    let mut out = Vec::new();
    let unfinished = |id: &'static str, tags: Vec<&'static str>, error: String| Judged {
        id,
        passed: None,
        critical: CRITICAL.contains(&id),
        detail: Value::Null,
        tags,
        error: Some(error),
    };
    let first_error = |tags: &[&str]| {
        tags.iter()
            .filter_map(|t| reply_for(round, replies, t))
            .find_map(|r| r.error.clone())
    };
    // Also unfinished: a filtered answer, even a partial one, or no answer
    // because the output cap or reasoning took it.
    let first_unanswered = |tags: &[&str]| {
        first_error(tags)
            .or_else(|| {
                tags.iter()
                    .filter_map(|t| reply_for(round, replies, t))
                    .find_map(|r| blocked(r.finish.as_deref()).map(str::to_owned))
            })
            .or_else(|| {
                tags.iter()
                    .filter_map(|t| reply_for(round, replies, t))
                    .filter(|r| {
                        r.text.as_deref().is_none_or(|t| t.trim().is_empty()) && r.tool.is_none()
                    })
                    .find_map(|r| unanswered(r.finish.as_deref(), r.reasoning).map(str::to_owned))
            })
    };
    if round.answers.len() == 2 {
        let tags = vec!["integrity_1", "integrity_2"];
        let texts: Vec<Option<&str>> = tags
            .iter()
            .map(|t| reply_for(round, replies, t).and_then(|r| r.text.as_deref()))
            .collect();
        if let Some(error) = first_unanswered(&tags) {
            out.push(unfinished("integrity", tags.clone(), error));
        } else {
            let checks: Vec<(bool, usize)> = texts
                .iter()
                .zip(&round.answers)
                .map(|(text, answer)| integrity_ok(text.unwrap_or(""), answer))
                .collect();
            let extra = checks.iter().map(|c| c.1).max().unwrap_or(0);
            out.push(Judged {
                id: "integrity",
                passed: Some(checks.iter().all(|c| c.0)),
                critical: true,
                detail: json!({"extra_chars":extra,"extra_text":extra > EXTRA_TEXT_WARNING}),
                tags: tags.clone(),
                error: None,
            });
        }
        // A replayed or cached answer carries the previous request's code.
        match (texts[1], first_unanswered(&tags[1..])) {
            (_, Some(error)) => out.push(unfinished("replay", vec!["integrity_2"], error)),
            (text, None) => {
                let text = text.unwrap_or("");
                let replayed = text.contains(round.answers[0].0.as_str())
                    || texts[0].is_some_and(|first| {
                        !first.trim().is_empty() && first.trim() == text.trim()
                    });
                out.push(Judged {
                    id: "replay",
                    passed: Some(!replayed),
                    critical: true,
                    detail: json!({"replayed":replayed}),
                    tags: vec!["integrity_2"],
                    error: None,
                });
            }
        }
    }
    let metering: Vec<(&Request, &Reply)> = round
        .requests
        .iter()
        .zip(replies)
        .filter(|(r, _)| r.tag.starts_with("metering"))
        .collect();
    if metering.len() == 2 {
        let tags = vec!["metering_short", "metering_long"];
        let exact: Option<Vec<(f64, f64)>> = metering
            .iter()
            .map(|(_, reply)| Some((reply.expected_input?, reply.input_tokens? as f64)))
            .collect();
        let points: Option<Vec<(f64, f64)>> = metering
            .iter()
            .map(|(request, reply)| {
                Some((
                    request.prompt.chars().count() as f64,
                    reply.input_tokens? as f64,
                ))
            })
            .collect();
        match (first_error(&tags), points) {
            (Some(error), _) => out.push(unfinished("metering", tags, error)),
            (None, None) => out.push(unfinished("metering", tags, "no_usage".into())),
            (None, Some(_)) if exact.is_some() => {
                let exact = exact.unwrap_or_default();
                out.push(Judged {
                    id: "metering",
                    critical: false,
                    tags,
                    error: None,
                    ..exact_metering(exact[0], exact[1])
                });
            }
            (None, Some(points)) => {
                let ((c1, t1), (c2, t2)) = (points[0], points[1]);
                let per_char = (t2 - t1) / (c2 - c1).max(1.0);
                let overhead = t1 - per_char * c1;
                let inflated = per_char > MAX_TOKENS_PER_CHAR;
                let hidden = overhead > MAX_FIXED_OVERHEAD;
                // Proportional inflation scales the slope and the overhead
                // alike, so without a local count only a gross slope shows.
                let reason = metering.iter().find_map(|(_, reply)| reply.uncounted);
                out.push(Judged {
                    id: "metering",
                    passed: Some(!inflated && !hidden),
                    critical: false,
                    detail: json!({"method":"estimate","inflation_checked":false,"reason":reason,
                        "tokens_per_char":(per_char * 1000.0).round() / 1000.0,
                        "fixed_overhead":overhead.round(),"inflated":inflated,"hidden_prompt":hidden}),
                    tags,
                    error: None,
                });
            }
        }
    }
    if let Some(reply) = reply_for(round, replies, "tools") {
        let tags = vec!["tools"];
        match first_unanswered(&tags) {
            Some(error) => out.push(unfinished("tools", tags, error)),
            None => {
                let called = reply.tool.as_ref().is_some_and(|(name, args)| {
                    name == TOOL_NAME && args["order_id"] == round.order_id.as_str()
                });
                out.push(Judged {
                    id: "tools",
                    passed: Some(called),
                    critical: false,
                    detail: json!({"tool_called":reply.tool.is_some()}),
                    tags,
                    error: None,
                });
            }
        }
    }
    if let (Some((lines, marker)), Some(reply)) =
        (&round.list, reply_for(round, replies, "truncation"))
    {
        let tags = vec!["truncation"];
        let text = reply.text.as_deref().unwrap_or("");
        let found: std::collections::HashSet<usize> = text
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                let digits: String = line.chars().take_while(char::is_ascii_digit).collect();
                let k: usize = digits.parse().ok()?;
                (line.contains(marker.as_str()) && (1..=*lines).contains(&k)).then_some(k)
            })
            .collect();
        let blank = text.trim().is_empty();
        let max_tokens = round
            .requests
            .iter()
            .find(|r| r.tag == "truncation")
            .map_or(TRUNCATION_MAX_TOKENS, |r| r.max_tokens);
        // Stopping at the cap well below what was asked for means the channel
        // lowered it; near the cap the model used the room itself.
        let capped = reply.output_tokens.map_or(!reply.reasoning, |t| {
            (t as f64) < f64::from(max_tokens) * 0.9
        });
        let outcome = match (&reply.error, reply.finish.as_deref()) {
            (Some(error), _) => Err(error.clone()),
            _ if found.len() >= *lines => Ok(true),
            // A blank answer would otherwise fail below.
            (None, Some(finish @ ("refusal" | "content_filter"))) => {
                Err(format!("quality_probe_finish_{finish}"))
            }
            (None, Some("length")) if capped => Ok(false),
            (None, Some("length")) => Err("quality_probe_truncated".to_owned()),
            (None, _) if blank && !reply.reasoning => Ok(false),
            (None, _) if blank => Err("quality_probe_reasoning_only".to_owned()),
            // A short list that ended normally is the model not following
            // the instruction, not a cap.
            (None, Some("stop")) => Ok(true),
            (None, None) => Err("quality_probe_finish_missing".to_owned()),
            (None, Some(other)) => Err(format!("quality_probe_finish_{}", other.to_lowercase())),
        };
        match outcome {
            Err(error) => out.push(unfinished("truncation", tags, error)),
            Ok(passed) => out.push(Judged {
                id: "truncation",
                passed: Some(passed),
                critical: false,
                detail: json!({"lines":found.len(),"expected":lines,"finish":reply.finish,
                    "output_tokens":reply.output_tokens}),
                tags,
                error: None,
            }),
        }
    }
    if let (Some(code), Some(reply)) = (&round.echo, reply_for(round, replies, "streaming")) {
        let tags = vec!["streaming"];
        match (first_unanswered(&tags), reply.stream) {
            (Some(error), _) => out.push(unfinished("streaming", tags, error)),
            (None, None) => out.push(unfinished("streaming", tags, "not_streamed".into())),
            (None, Some(report)) => {
                let echoed = reply
                    .text
                    .as_deref()
                    .is_some_and(|t| t.contains(code.as_str()));
                out.push(Judged {
                    id: "streaming",
                    passed: Some(report.events_ok && report.ended && echoed),
                    critical: false,
                    detail: json!({"events_ok":report.events_ok,"ended":report.ended,
                        "echoed":echoed,"usage_reported":report.usage_reported}),
                    tags,
                    error: None,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{SeedableRng, rngs::StdRng};

    fn round() -> Round {
        plan(&METHODS.map(str::to_owned), &mut StdRng::seed_from_u64(3))
    }

    fn honest(round: &Round) -> Vec<Reply> {
        let mut answers = round.answers.iter();
        round
            .requests
            .iter()
            .map(|r| match r.tag {
                "integrity_1" | "integrity_2" => {
                    let (code, sum) = answers.next().unwrap();
                    Reply {
                        text: Some(format!("{code}\n{sum}")),
                        input_tokens: Some(40),
                        ..Default::default()
                    }
                }
                "tools" => Reply {
                    tool: Some((TOOL_NAME.into(), json!({"order_id":round.order_id}))),
                    input_tokens: Some(60),
                    ..Default::default()
                },
                "truncation" => {
                    let (lines, marker) = round.list.as_ref().unwrap();
                    Reply {
                        text: Some((1..=*lines).map(|k| format!("{k}. {marker}\n")).collect()),
                        finish: Some("stop".into()),
                        ..Default::default()
                    }
                }
                "streaming" => Reply {
                    text: round.echo.clone(),
                    stream: Some(StreamReport {
                        events_ok: true,
                        ended: true,
                        usage_reported: true,
                    }),
                    ..Default::default()
                },
                _ => Reply {
                    text: Some("OK".into()),
                    input_tokens: Some((r.prompt.chars().count() as f64 * 0.3) as i64 + 12),
                    ..Default::default()
                },
            })
            .collect()
    }

    fn verdicts(judged: &[Judged]) -> Vec<(&str, Option<bool>)> {
        judged.iter().map(|j| (j.id, j.passed)).collect()
    }

    #[test]
    fn honest_channel_passes_every_check() {
        let round = round();
        assert_eq!(round.requests.len(), 7);
        assert_eq!(
            verdicts(&judge(&round, &honest(&round))),
            vec![
                ("integrity", Some(true)),
                ("replay", Some(true)),
                ("metering", Some(true)),
                ("tools", Some(true)),
                ("truncation", Some(true)),
                ("streaming", Some(true))
            ]
        );
    }

    #[test]
    fn calibrated_models_count_their_check_requests() {
        let round = round();
        let count = |protocol, model| {
            round
                .requests
                .iter()
                .map(|r| {
                    let (path, body) = body(protocol, model, r);
                    crate::token_estimate::request_tokens(&path, &body, model)
                })
                .collect::<Vec<_>>()
        };
        assert!(
            count("anthropic", "claude-sonnet-5-5")
                .iter()
                .all(Result::is_ok)
        );
        assert!(count("openai", "gpt-6.1-sol").iter().all(Result::is_ok));
        assert!(
            count("openai", "claude-sonnet-5-5")
                .iter()
                .all(|r| *r == Err("converted_protocol"))
        );
        assert!(
            count("gemini", "gemini-3-pro")
                .iter()
                .all(|r| *r == Err("uncalibrated"))
        );
    }

    #[test]
    fn local_counts_catch_small_hidden_prompts_and_inflation() {
        let round = round();
        let counted = |extra: &dyn Fn(f64) -> f64| {
            let mut replies = honest(&round);
            for (request, reply) in round.requests.iter().zip(&mut replies) {
                if request.tag.starts_with("metering") {
                    let expected = request.prompt.chars().count() as f64 * 0.3 + 10.0;
                    reply.expected_input = Some(expected);
                    reply.input_tokens = Some(extra(expected).round() as i64);
                }
            }
            judge(&round, &replies).remove(2)
        };
        let honest = counted(&|e| e + 3.0);
        assert_eq!(
            (honest.passed, &honest.detail["method"]),
            (Some(true), &json!("exact"))
        );
        // Far below the 300-token overhead the two-point fit allows.
        let hidden = counted(&|e| e + 120.0);
        assert_eq!(hidden.passed, Some(false));
        assert_eq!(hidden.detail["hidden_prompt"], true);
        assert_eq!(hidden.detail["inflated"], false);
        let inflated = counted(&|e| e * 1.3);
        assert_eq!(inflated.passed, Some(false));
        assert_eq!(inflated.detail["hidden_prompt"], false);
        assert_eq!(inflated.detail["inflated"], true);
    }

    #[test]
    fn tampering_replay_metering_and_tools_fail() {
        let round = round();
        let mut replies = honest(&round);
        // The second answer replays the first.
        replies[1].text = replies[0].text.clone();
        let judged = judge(&round, &replies);
        assert_eq!(judged[0].passed, Some(false));
        assert_eq!(judged[1].passed, Some(false));
        // An ad appended to the answer is reported but not a failure on its own.
        let mut replies = honest(&round);
        replies[0].text = Some(format!(
            "{}\n{}",
            replies[0].text.as_deref().unwrap(),
            "限时优惠！".repeat(40)
        ));
        let judged = judge(&round, &replies);
        assert_eq!(judged[0].passed, Some(true));
        assert_eq!(judged[0].detail["extra_text"], true);
        // A hidden system prompt adds a large fixed overhead; inflation a steep slope.
        let mut replies = honest(&round);
        for r in &mut replies[2..4] {
            r.input_tokens = r.input_tokens.map(|t| t + 2000);
        }
        let metering = &judge(&round, &replies)[2];
        assert_eq!(metering.passed, Some(false));
        assert_eq!(metering.detail["hidden_prompt"], true);
        assert_eq!(metering.detail["method"], "estimate");
        assert_eq!(metering.detail["inflation_checked"], false);
        // 150 tokens on every request now exceed the estimate's limit too.
        let mut replies = honest(&round);
        for r in &mut replies[2..4] {
            r.input_tokens = r.input_tokens.map(|t| t + 150);
            r.uncounted = Some("uncalibrated");
        }
        let metering = &judge(&round, &replies)[2];
        assert_eq!(metering.detail["hidden_prompt"], true);
        assert_eq!(metering.detail["reason"], "uncalibrated");
        let mut replies = honest(&round);
        for (r, req) in replies[2..4].iter_mut().zip(&round.requests[2..4]) {
            r.input_tokens = Some(req.prompt.chars().count() as i64 * 2);
        }
        assert_eq!(judge(&round, &replies)[2].detail["inflated"], true);
        // Text instead of a tool call fails; transport errors leave checks unfinished.
        let mut replies = honest(&round);
        replies[4] = Reply {
            text: Some("订单正在处理".into()),
            ..Default::default()
        };
        replies[0].error = Some("probe_http_502".into());
        let judged = judge(&round, &replies);
        assert_eq!(judged[0].passed, None);
        assert_eq!(judged[1].passed, Some(true));
        assert_eq!(judged[3].passed, Some(false));
    }

    #[test]
    fn cut_lists_and_broken_streams_fail() {
        let round = round();
        // Cut off at the output cap.
        let mut replies = honest(&round);
        replies[5].text = Some(
            replies[5]
                .text
                .as_deref()
                .unwrap()
                .lines()
                .take(12)
                .collect::<Vec<_>>()
                .join("\n"),
        );
        replies[5].finish = Some("length".into());
        let judged = judge(&round, &replies);
        assert_eq!(judged[4].passed, Some(false));
        assert_eq!(judged[4].detail["lines"], 12);
        // A short list that ended normally is not proof of a cap.
        replies[5].finish = Some("stop".into());
        assert_eq!(judge(&round, &replies)[4].passed, Some(true));
        // A stream that never ends fails; missing usage is only reported.
        let mut replies = honest(&round);
        replies[6].stream = Some(StreamReport {
            events_ok: true,
            ended: false,
            usage_reported: false,
        });
        let streaming = &judge(&round, &replies)[5];
        assert_eq!(streaming.passed, Some(false));
        assert_eq!(streaming.detail["usage_reported"], false);
    }

    #[test]
    fn streams_parse_in_every_protocol() {
        let openai = b"data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"AB\"}}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2}}\n\ndata: [DONE]\n";
        let Streamed { text, report, .. } = parse_stream("openai", openai);
        assert_eq!(text.as_deref(), Some("AB"));
        assert_eq!(
            report,
            StreamReport {
                events_ok: true,
                ended: true,
                usage_reported: true
            }
        );
        let anthropic = b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"AB\"}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n";
        let Streamed { text, report, .. } = parse_stream("anthropic", anthropic);
        assert_eq!(text.as_deref(), Some("AB"));
        assert!(report.events_ok && report.ended && !report.usage_reported);
        let gemini = b"data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"AB\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":4,\"candidatesTokenCount\":1}}\n";
        let streamed = parse_stream("gemini", gemini);
        assert_eq!(streamed.text.as_deref(), Some("AB"));
        assert_eq!(streamed.finish.as_deref(), Some("stop"));
        let report = streamed.report;
        assert!(report.events_ok && report.ended && report.usage_reported);
        let broken = parse_stream("openai", b"data: {not json\n").report;
        assert!(!broken.events_ok && !broken.ended);
        // Reasoning is noticed but kept out of the answer, with the stop reason.
        let openai = b"data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"hmm\"}}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}]}\n\ndata: [DONE]\n";
        let streamed = parse_stream("openai", openai);
        assert!(streamed.text.is_none() && streamed.reasoning);
        assert_eq!(streamed.finish.as_deref(), Some("length"));
        let anthropic = b"data: {\"type\":\"content_block_start\",\"content_block\":{\"type\":\"thinking\"}}\n\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"hmm\"}}\n\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"max_tokens\"}}\n\ndata: {\"type\":\"message_stop\"}\n";
        let streamed = parse_stream("anthropic", anthropic);
        assert!(streamed.text.is_none() && streamed.reasoning);
        assert_eq!(streamed.finish.as_deref(), Some("length"));
        let gemini = b"data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"plan\",\"thought\":true},{\"text\":\"AB\"}]},\"finishReason\":\"STOP\"}]}\n";
        let streamed = parse_stream("gemini", gemini);
        assert_eq!(streamed.text.as_deref(), Some("AB"));
        assert!(streamed.reasoning);
    }

    #[test]
    fn refused_answers_are_unfinished() {
        let round = round();
        // A blank refusal on every chat check, and a partial one on integrity.
        let mut replies = honest(&round);
        for i in [0, 5, 6] {
            replies[i].text = None;
            replies[i].finish = Some("refusal".into());
        }
        replies[1].text = Some(round.answers[1].0.clone());
        replies[1].finish = Some("refusal".into());
        let judged = judge(&round, &replies);
        for i in [0, 1, 4, 5] {
            assert_eq!(judged[i].passed, None, "{}", judged[i].id);
            assert_eq!(
                judged[i].error.as_deref(),
                Some("quality_probe_finish_refusal")
            );
        }
        // Gemini's safety stops read as a content filter.
        assert_eq!(normalize_finish("SAFETY"), "content_filter");
        assert_eq!(
            unanswered(Some("content_filter"), false),
            Some("quality_probe_finish_content_filter")
        );
    }

    #[test]
    fn check_phrases_are_letters_only() {
        let round = round();
        for (code, _) in &round.answers {
            assert_eq!(code.split('-').count(), 4);
            assert!(code.chars().all(|c| c == '-' || c.is_ascii_lowercase()));
        }
        assert_ne!(round.answers[0].0, round.answers[1].0);
    }

    #[test]
    fn blank_answers_fail_and_capped_or_reasoning_ones_are_unfinished() {
        let round = round();
        // A blank 200 fails integrity, tools and streaming.
        let mut replies = honest(&round);
        for i in [0, 4, 6] {
            replies[i].text = None;
            replies[i].tool = None;
            replies[i].finish = Some("stop".into());
        }
        let judged = judge(&round, &replies);
        assert_eq!(judged[0].passed, Some(false));
        assert_eq!(judged[3].passed, Some(false));
        assert_eq!(judged[5].passed, Some(false));
        // Reasoning that used up the cap leaves the same checks unfinished.
        let mut replies = honest(&round);
        replies[1].text = None;
        replies[1].finish = Some("length".into());
        replies[4].tool = None;
        replies[4].reasoning = true;
        replies[6].text = None;
        replies[6].reasoning = true;
        let judged = judge(&round, &replies);
        assert_eq!(judged[0].passed, None);
        assert_eq!(judged[0].error.as_deref(), Some("quality_probe_truncated"));
        assert_eq!(judged[1].passed, None);
        assert_eq!(
            judged[3].error.as_deref(),
            Some("quality_probe_reasoning_only")
        );
        assert_eq!(judged[5].passed, None);
        // Metering only needs usage, whatever the answer.
        let mut replies = honest(&round);
        replies[2].text = None;
        replies[2].finish = Some("length".into());
        assert_eq!(judge(&round, &replies)[2].passed, Some(true));
    }

    #[test]
    fn truncation_fails_only_on_a_lowered_cap() {
        let round = round();
        let short = |replies: &mut Vec<Reply>, finish: Option<&str>, output: Option<i64>| {
            replies[5].text = Some(
                replies[5]
                    .text
                    .as_deref()
                    .unwrap()
                    .lines()
                    .take(5)
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            replies[5].finish = finish.map(str::to_owned);
            replies[5].output_tokens = output;
        };
        let truncation = |replies: &[Reply]| {
            let judged = judge(&round, replies);
            (judged[4].passed, judged[4].error.clone())
        };
        let mut replies = honest(&round);
        short(&mut replies, Some("length"), Some(300));
        assert_eq!(truncation(&replies), (Some(false), None));
        // The model used the whole cap itself.
        let mut replies = honest(&round);
        short(&mut replies, Some("length"), Some(4096));
        assert_eq!(
            truncation(&replies),
            (None, Some("quality_probe_truncated".into()))
        );
        // Nothing at all under a lowered cap still fails.
        let mut replies = honest(&round);
        replies[5].text = None;
        replies[5].finish = Some("length".into());
        replies[5].output_tokens = Some(0);
        assert_eq!(truncation(&replies), (Some(false), None));
        // A missing or unusual stop reason is not proof of a cap.
        let mut replies = honest(&round);
        short(&mut replies, None, Some(60));
        assert_eq!(
            truncation(&replies),
            (None, Some("quality_probe_finish_missing".into()))
        );
        let mut replies = honest(&round);
        short(&mut replies, Some("content_filter"), Some(60));
        assert_eq!(
            truncation(&replies),
            (None, Some("quality_probe_finish_content_filter".into()))
        );
    }

    #[test]
    fn sums_with_thousands_separators_count() {
        let answer = ("ABCDEFGHJKLMNPQRSTUVWXYZ".to_owned(), 1234);
        assert!(integrity_ok("ABCDEFGHJKLMNPQRSTUVWXYZ\n1,234", &answer).0);
        assert!(integrity_ok("ABCDEFGHJKLMNPQRSTUVWXYZ\n1234", &answer).0);
        assert!(!integrity_ok("ABCDEFGHJKLMNPQRSTUVWXYZ\n1,2345", &answer).0);
        assert_eq!(
            without_separators("12,345,678 and 1, 2"),
            "12345678 and 1, 2"
        );
    }

    #[test]
    fn answers_and_reasoning_are_read_in_every_protocol() {
        let anthropic = json!({"content":[{"type":"thinking","thinking":"x"},{"type":"text","text":"A"},{"type":"text","text":"B"}]});
        assert_eq!(
            crate::monitor::answer_text("anthropic", &anthropic).as_deref(),
            Some("AB")
        );
        assert!(reasoning("anthropic", &anthropic));
        let gemini = json!({"candidates":[{"content":{"parts":[{"text":"plan","thought":true},{"text":"A"},{"text":"B"}]}}]});
        assert_eq!(
            crate::monitor::answer_text("gemini", &gemini).as_deref(),
            Some("AB")
        );
        assert!(reasoning("gemini", &gemini));
        let openai = json!({"choices":[{"message":{"content":null,"reasoning_content":"hmm"},"finish_reason":"length"}]});
        assert!(crate::monitor::answer_text("openai", &openai).is_none());
        assert!(reasoning("openai", &openai));
        assert!(!reasoning(
            "openai",
            &json!({"choices":[{"message":{"content":""}}]})
        ));
        assert!(crate::monitor::answer_text("anthropic", &json!({"content":[]})).is_none());
    }

    #[test]
    fn tool_calls_parse_in_every_protocol() {
        let openai = json!({"choices":[{"message":{"tool_calls":[{"function":{"name":"lookup_order","arguments":"{\"order_id\":\"ORD-1\"}"}}]}}]});
        let anthropic = json!({"content":[{"type":"text","text":"ok"},{"type":"tool_use","name":"lookup_order","input":{"order_id":"ORD-1"}}]});
        let gemini = json!({"candidates":[{"content":{"parts":[{"functionCall":{"name":"lookup_order","args":{"order_id":"ORD-1"}}}]}}]});
        for (protocol, payload) in [
            ("openai", openai),
            ("anthropic", anthropic),
            ("gemini", gemini),
        ] {
            let (name, args) = tool_call(protocol, &payload).unwrap();
            assert_eq!(
                (name.as_str(), &args["order_id"]),
                ("lookup_order", &json!("ORD-1"))
            );
        }
        assert!(tool_call("openai", &json!({"choices":[{"message":{"content":"hi"}}]})).is_none());
    }

    #[test]
    fn rounds_are_randomized() {
        let a = plan(&METHODS.map(str::to_owned), &mut StdRng::seed_from_u64(1));
        let b = plan(&METHODS.map(str::to_owned), &mut StdRng::seed_from_u64(2));
        assert_ne!(a.requests[0].prompt, b.requests[0].prompt);
        assert_ne!(a.order_id, b.order_id);
        assert!(
            plan(&["tools".into()], &mut StdRng::seed_from_u64(1))
                .requests
                .len()
                == 1
        );
    }
}
