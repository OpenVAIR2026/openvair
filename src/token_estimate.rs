//! Local token counts of request text, for passive metering against what the
//! upstream reports.
//!
//! A request is split once into text segments by role (prose, tool traffic,
//! thinking, tool definitions) plus structural counts; each candidate
//! tokenizer then counts the segments. The message frame is not added here:
//! [`Shape`] keeps the parts separate so frame constants can be calibrated, and
//! recalibrated, from stored shapes without the request text.

use futures_util::FutureExt;
use futures_util::future::{BoxFuture, Shared};
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;

/// Shapes of one request under each tokenizer its routes may use, counted on
/// a blocking thread while the request is upstream.
pub type Shapes = Shared<BoxFuture<'static, Option<Arc<Vec<Shape>>>>>;

/// Starts counting a request for the tokenizers of `models`; None when none
/// of them has a known tokenizer or the request cannot be counted.
pub fn spawn<'a>(
    path: &str,
    payload: &Value,
    models: impl IntoIterator<Item = &'a str>,
) -> Option<Shapes> {
    let mut tokenizers: Vec<Tokenizer> = Vec::new();
    for tokenizer in models.into_iter().filter_map(Tokenizer::for_model) {
        if !tokenizers.contains(&tokenizer) {
            tokenizers.push(tokenizer);
        }
    }
    if tokenizers.is_empty() {
        return None;
    }
    let segments = extract(path, payload)?;
    let task = tokio::task::spawn_blocking(move || {
        tokenizers
            .into_iter()
            .map(|t| segments.count(t))
            .collect::<Vec<_>>()
    });
    Some(
        task.map(|shapes| shapes.ok().map(Arc::new))
            .boxed()
            .shared(),
    )
}

/// The shape counted with `model`'s tokenizer, as stored in usage_events.
pub fn shape_for(shapes: &[Shape], model: &str) -> Option<Value> {
    let name = Tokenizer::for_model(model)?.name();
    let shape = shapes.iter().find(|s| s.tokenizer == name)?;
    serde_json::to_value(shape).ok()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tokenizer {
    /// Claude Opus 4.8, Sonnet 5, Fable 5 and later (ctok's v4.7 vocabulary).
    ClaudeV48,
    /// GPT-5 and GPT-6 models, confirmed against authenticity baselines.
    O200k,
}

impl Tokenizer {
    /// The tokenizer an upstream model is known to use, or None when unverified.
    pub fn for_model(model: &str) -> Option<Self> {
        let model = model
            .rsplit('/')
            .next()
            .unwrap_or(model)
            .to_ascii_lowercase();
        if let Some(rest) = model.strip_prefix("claude-") {
            // Names are `claude-<tier>-<major>[-<minor>]`, e.g. claude-opus-4-8.
            let mut numbers = rest.split(['-', '.']).filter_map(|p| p.parse::<u32>().ok());
            let major = numbers.next()?;
            let minor = numbers.next().filter(|m| *m < 100).unwrap_or(0);
            return (major >= 5 || (major == 4 && minor >= 8)).then_some(Self::ClaudeV48);
        }
        (model.starts_with("gpt-5") || model.starts_with("gpt-6")).then_some(Self::O200k)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::ClaudeV48 => "claude-v4.8",
            Self::O200k => "o200k",
        }
    }

    fn count(self, text: &str) -> u64 {
        match self {
            Self::ClaudeV48 => crate::ctok::content_tokens(text) as u64,
            Self::O200k => tiktoken_rs::o200k_base_singleton()
                .encode_ordinary(text)
                .len() as u64,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Text,
    /// Tool calls and their results.
    Tool,
    Thinking,
    /// Response schemas, serialized as JSON.
    Definition,
}

/// Request text by part, with the structure the frame depends on.
#[derive(Debug, Default)]
pub struct Segments {
    parts: Vec<(Part, String)>,
    /// Tool definitions, rendered per tokenizer the way its vendor shows them
    /// to the model.
    tools: Vec<Value>,
    system: bool,
    /// Messages other than the system prompt.
    messages: u32,
    blocks: u32,
    /// Tool call and tool result blocks.
    calls: u32,
}

/// Token counts of one request under one tokenizer, before the frame.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Shape {
    pub tokenizer: &'static str,
    pub text: u64,
    pub tool: u64,
    pub thinking: u64,
    pub definitions: u64,
    pub system: bool,
    pub messages: u32,
    pub blocks: u32,
    pub tools: u32,
    pub calls: u32,
}

impl Segments {
    pub fn count(&self, tokenizer: Tokenizer) -> Shape {
        let mut shape = Shape {
            tokenizer: tokenizer.name(),
            system: self.system,
            messages: self.messages,
            blocks: self.blocks,
            tools: self.tools.len() as u32,
            calls: self.calls,
            ..Shape::default()
        };
        for tool in &self.tools {
            let rendered = match tokenizer {
                Tokenizer::O200k => typescript_tool(tool),
                Tokenizer::ClaudeV48 => None,
            };
            shape.definitions += tokenizer.count(&rendered.unwrap_or_else(|| tool.to_string()));
        }
        for (part, text) in &self.parts {
            let tokens = tokenizer.count(text);
            match part {
                Part::Text => shape.text += tokens,
                Part::Tool => shape.tool += tokens,
                Part::Thinking => shape.thinking += tokens,
                Part::Definition => shape.definitions += tokens,
            }
        }
        shape
    }

    fn push(&mut self, part: Part, text: &str) {
        if !text.is_empty() {
            self.parts.push((part, text.to_owned()));
        }
    }

    fn push_json(&mut self, part: Part, value: &Value) {
        match value {
            Value::String(text) => self.push(part, text),
            Value::Null => {}
            other => self.push(part, &other.to_string()),
        }
    }

    /// Function arguments as the model sees them: OpenAI parses the JSON
    /// string, so escapes such as `\uXXXX` cost their characters, not their
    /// spelling.
    fn push_arguments(&mut self, arguments: &Value) {
        match arguments
            .as_str()
            .and_then(|a| serde_json::from_str::<Value>(a).ok())
        {
            Some(parsed) => self.push_json(Part::Tool, &parsed),
            None => self.push_json(Part::Tool, arguments),
        }
    }

    fn definitions(&mut self, payload: &Value, keys: &[&str]) -> Option<()> {
        for key in keys {
            match payload.get(*key) {
                None | Some(Value::Null) => {}
                Some(Value::Array(tools)) if *key == "tools" => {
                    self.tools.extend(tools.iter().cloned());
                }
                Some(other) => self.push_json(Part::Definition, other),
            }
        }
        Some(())
    }
}

/// Splits a request into countable segments, or None for unsupported
/// endpoints and content whose tokens text cannot predict (images, files,
/// encrypted reasoning, unknown block types).
pub fn extract(path: &str, payload: &Value) -> Option<Segments> {
    let mut out = Segments::default();
    match path {
        "/v1/messages" => anthropic(payload, &mut out)?,
        "/v1/chat/completions" => chat(payload, &mut out)?,
        "/v1/responses" => responses(payload, &mut out)?,
        _ => return None,
    }
    Some(out)
}

fn anthropic(payload: &Value, out: &mut Segments) -> Option<()> {
    match payload.get("system") {
        None | Some(Value::Null) => {}
        Some(Value::String(text)) => {
            out.system = true;
            out.push(Part::Text, text);
        }
        Some(Value::Array(blocks)) => {
            out.system = true;
            for block in blocks {
                out.push(Part::Text, text_block(block)?);
            }
        }
        Some(_) => return None,
    }
    for message in payload.get("messages")?.as_array()? {
        out.messages += 1;
        match message.get("content")? {
            Value::String(text) => {
                out.blocks += 1;
                out.push(Part::Text, text);
            }
            Value::Array(blocks) => {
                for block in blocks {
                    out.blocks += 1;
                    anthropic_block(block, out)?;
                }
            }
            _ => return None,
        }
    }
    out.definitions(payload, &["tools"])
}

fn anthropic_block(block: &Value, out: &mut Segments) -> Option<()> {
    match block.get("type")?.as_str()? {
        "text" => out.push(Part::Text, block.get("text")?.as_str()?),
        "thinking" => out.push(Part::Thinking, block.get("thinking")?.as_str()?),
        "tool_use" | "server_tool_use" => {
            out.calls += 1;
            out.push(Part::Tool, block.get("name")?.as_str()?);
            out.push_json(Part::Tool, block.get("input")?);
        }
        "tool_result" => {
            out.calls += 1;
            match block.get("content") {
                None | Some(Value::Null) => {}
                Some(Value::String(text)) => out.push(Part::Tool, text),
                Some(Value::Array(items)) => {
                    for item in items {
                        out.push(Part::Tool, text_block(item)?);
                    }
                }
                Some(_) => return None,
            }
        }
        _ => return None,
    }
    Some(())
}

/// The text of a `{"type":"text","text":...}` block; None for any other type.
fn text_block(block: &Value) -> Option<&str> {
    matches!(
        block.get("type")?.as_str()?,
        "text" | "input_text" | "output_text"
    )
    .then(|| block.get("text")?.as_str())?
}

fn chat(payload: &Value, out: &mut Segments) -> Option<()> {
    for message in payload.get("messages")?.as_array()? {
        let role = message.get("role")?.as_str()?;
        let content = message.get("content").filter(|c| !c.is_null());
        // The first system message frames like a top-level system prompt, and
        // tool calls and results frame as calls, as in the Responses API.
        if matches!(role, "system" | "developer") && !out.system {
            out.system = true;
        } else if role != "tool" && content.is_some() {
            out.messages += 1;
        }
        let part = if role == "tool" {
            out.calls += 1;
            Part::Tool
        } else {
            Part::Text
        };
        match content {
            None => {}
            Some(Value::String(text)) => {
                out.blocks += 1;
                out.push(part, text);
            }
            Some(Value::Array(items)) => {
                for item in items {
                    out.blocks += 1;
                    out.push(part, text_block(item)?);
                }
            }
            Some(_) => return None,
        }
        if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                out.blocks += 1;
                out.calls += 1;
                let function = call.get("function")?;
                out.push(Part::Tool, function.get("name")?.as_str()?);
                out.push_arguments(function.get("arguments")?);
            }
        }
    }
    out.definitions(payload, &["tools", "response_format"])
}

fn responses(payload: &Value, out: &mut Segments) -> Option<()> {
    if let Some(instructions) = payload.get("instructions").and_then(Value::as_str) {
        out.system = true;
        out.push(Part::Text, instructions);
    }
    match payload.get("input")? {
        Value::String(text) => {
            out.messages += 1;
            out.blocks += 1;
            out.push(Part::Text, text);
        }
        Value::Array(items) => {
            for item in items {
                responses_item(item, out)?;
            }
        }
        _ => return None,
    }
    let format = payload
        .pointer("/text/format")
        .filter(|f| f.get("type").and_then(Value::as_str) == Some("json_schema"));
    if let Some(format) = format {
        out.push_json(Part::Definition, format);
    }
    out.definitions(payload, &["tools"])
}

fn responses_item(item: &Value, out: &mut Segments) -> Option<()> {
    let kind = match item.get("type").and_then(Value::as_str) {
        Some(kind) => kind,
        None if item.get("role").is_some() => "message",
        None => return None,
    };
    match kind {
        "message" => {
            if matches!(
                item.get("role").and_then(Value::as_str),
                Some("system" | "developer")
            ) && !out.system
            {
                out.system = true;
            } else {
                out.messages += 1;
            }
            match item.get("content")? {
                Value::String(text) => {
                    out.blocks += 1;
                    out.push(Part::Text, text);
                }
                Value::Array(parts) => {
                    for part in parts {
                        out.blocks += 1;
                        out.push(Part::Text, text_block(part)?);
                    }
                }
                _ => return None,
            }
        }
        "function_call" | "custom_tool_call" => {
            out.blocks += 1;
            out.calls += 1;
            out.push(Part::Tool, item.get("name")?.as_str()?);
            out.push_arguments(item.get("arguments").or_else(|| item.get("input"))?);
        }
        "function_call_output" | "custom_tool_call_output" => {
            out.blocks += 1;
            out.calls += 1;
            match item.get("output")? {
                Value::Array(parts) => {
                    for part in parts {
                        out.push(Part::Tool, text_block(part)?);
                    }
                }
                other => out.push_json(Part::Tool, other),
            }
        }
        "reasoning" => {
            if item.get("encrypted_content").is_some_and(|v| !v.is_null()) {
                return None;
            }
            out.blocks += 1;
            for summary in item
                .get("summary")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                out.push(Part::Thinking, summary.get("text")?.as_str()?);
            }
        }
        _ => return None,
    }
    Some(())
}

/// A function tool as OpenAI shows it to the model: a TypeScript declaration
/// in the harmony `functions` namespace. None for other tool types, which are
/// counted as JSON.
fn typescript_tool(tool: &Value) -> Option<String> {
    let function = tool.get("function").unwrap_or(tool);
    if tool
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|t| t != "function")
    {
        return None;
    }
    let name = function.get("name")?.as_str()?;
    let mut out = String::new();
    comment(&mut out, function.get("description"));
    let parameters = function.get("parameters");
    if parameters
        .and_then(|p| p.get("properties"))
        .and_then(Value::as_object)
        .is_some_and(|p| !p.is_empty())
    {
        out += &format!(
            "type {name} = (_: {}) => any;\n\n",
            typescript_object(parameters?)?
        );
    } else {
        out += &format!("type {name} = () => any;\n\n");
    }
    Some(out)
}

fn comment(out: &mut String, description: Option<&Value>) {
    for line in description.and_then(Value::as_str).unwrap_or("").lines() {
        *out += "// ";
        *out += line;
        *out += "\n";
    }
}

fn typescript_object(schema: &Value) -> Option<String> {
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut out = String::from("{\n");
    for (key, property) in schema.get("properties")?.as_object()? {
        comment(&mut out, property.get("description"));
        let optional = if required.contains(&key.as_str()) {
            ""
        } else {
            "?"
        };
        out += &format!("{key}{optional}: {},\n", typescript_type(property)?);
    }
    out += "}";
    Some(out)
}

fn typescript_type(schema: &Value) -> Option<String> {
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        let values: Vec<String> = values.iter().map(Value::to_string).collect();
        return Some(values.join(" | "));
    }
    if let Some(options) = schema.get("anyOf").and_then(Value::as_array) {
        let options: Option<Vec<String>> = options.iter().map(typescript_type).collect();
        return Some(options?.join(" | "));
    }
    Some(match schema.get("type").and_then(Value::as_str) {
        Some("string") => "string".into(),
        Some("integer" | "number") => "number".into(),
        Some("boolean") => "boolean".into(),
        Some("null") => "null".into(),
        Some("array") => match schema.get("items") {
            Some(items) => format!("{}[]", typescript_type(items)?),
            None => "any[]".into(),
        },
        Some("object") if schema.get("properties").is_some() => typescript_object(schema)?,
        _ => "any".into(),
    })
}

/// Tokens a request costs beyond its content: role markers, call delimiters
/// and the vendor's own tool-use instructions.
struct Frame {
    base: f64,
    system: f64,
    message: f64,
    call: f64,
    /// Added once when the request defines any tool.
    tools: f64,
    tool: f64,
}

/// Whether the frames below are fitted against official counts.
pub const FRAMES_CALIBRATED: bool = true;

/// Frames fitted on 2026-10-21 against official counts of synthetic requests
/// (text, system prompts, multi-turn, multi-block, tool definitions and tool
/// rounds): within 8 tokens, or 1.7%, for Claude Opus 4.8 to 5.5 and GPT-5.6
/// to 6.1 on Messages, Chat Completions and Responses.
fn frame(tokenizer: &str, model: &str) -> Option<Frame> {
    match tokenizer {
        "claude-v4.8" => {
            let model = model.rsplit('/').next().unwrap_or(model).replace('.', "-");
            // Opus 4.8 and the 5.0 models open with two fewer tokens; Sonnet 5
            // carries a longer tool-use prompt.
            let early = ["opus-4-8", "sonnet-5", "fable-5"]
                .iter()
                .any(|m| model.ends_with(m));
            Some(Frame {
                base: if early { 4.0 } else { 6.0 },
                system: 1.0,
                message: 2.0,
                call: 19.0,
                tools: if model.ends_with("sonnet-5") {
                    356.0
                } else {
                    288.0
                },
                tool: 12.0,
            })
        }
        "o200k" => Some(Frame {
            base: 1.0,
            system: 4.0,
            message: 5.0,
            call: 11.0,
            tools: 15.0,
            tool: -2.0,
        }),
        _ => None,
    }
}

/// Models whose frames were fitted against official counts. Other models,
/// even of a covered tokenizer, may carry a different frame or tool prompt.
const CALIBRATED_MODELS: [&str; 10] = [
    "claude-opus-5-5",
    "claude-sonnet-5-5",
    "claude-haiku-5-5",
    "claude-fable-5-1",
    "claude-opus-4-8",
    "claude-sonnet-5",
    "claude-fable-5",
    "gpt-6-1-sol",
    "gpt-6-astra",
    "gpt-5-6-sol",
];

/// Whether `model` is a calibrated model, ignoring a vendor prefix, case,
/// dots for dashes and a trailing date snapshot.
pub fn calibrated(model: &str) -> bool {
    let name = model
        .rsplit('/')
        .next()
        .unwrap_or(model)
        .to_ascii_lowercase()
        .replace('.', "-");
    let name = match name.rsplit_once('-') {
        Some((stem, date)) if date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()) => stem,
        _ => &name,
    };
    CALIBRATED_MODELS.contains(&name)
}

/// Input tokens one request should cost on `model`, counted now, for a
/// calibrated model on its native API: Claude on Messages, GPT on Chat
/// Completions and Responses. Otherwise why it was not counted:
/// `uncalibrated`, `converted_protocol` or `uncountable`.
pub fn request_tokens(path: &str, payload: &Value, model: &str) -> Result<f64, &'static str> {
    let tokenizer = Tokenizer::for_model(model)
        .filter(|_| calibrated(model))
        .ok_or("uncalibrated")?;
    let native = match tokenizer {
        Tokenizer::ClaudeV48 => path == "/v1/messages",
        Tokenizer::O200k => matches!(path, "/v1/chat/completions" | "/v1/responses"),
    };
    if !native {
        return Err("converted_protocol");
    }
    extract(path, payload)
        .and_then(|segments| serde_json::to_value(segments.count(tokenizer)).ok())
        .and_then(|shape| expected_tokens(&shape, model))
        .ok_or("uncountable")
}

/// Input tokens a stored shape should cost on `model`, frame included; None
/// for models that are not calibrated.
pub fn expected_tokens(shape: &Value, model: &str) -> Option<f64> {
    if !calibrated(model) {
        return None;
    }
    let frame = frame(shape.get("tokenizer")?.as_str()?, model)?;
    let number = |key: &str| shape.get(key).and_then(Value::as_f64).unwrap_or(0.0);
    let content = number("text") + number("tool") + number("thinking") + number("definitions");
    let system = shape.get("system").and_then(Value::as_bool) == Some(true);
    let tools = number("tools");
    Some(
        content
            + frame.base
            + if system { frame.system } else { 0.0 }
            + frame.message * number("messages")
            + frame.call * number("calls")
            + if tools > 0.0 {
                frame.tools + frame.tool * tools
            } else {
                0.0
            },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tokenizer_follows_verified_model_families() {
        for model in [
            "claude-sonnet-5-5",
            "claude-opus-4-8",
            "claude-fable-5-1",
            "anthropic/claude-opus-5-5",
            "claude-sonnet-5-20261001",
        ] {
            assert_eq!(
                Tokenizer::for_model(model),
                Some(Tokenizer::ClaudeV48),
                "{model}"
            );
        }
        for model in [
            "claude-opus-4-6",
            "claude-3-5-sonnet",
            "claude-sonnet-4-20250514",
        ] {
            assert_eq!(Tokenizer::for_model(model), None, "{model}");
        }
        assert_eq!(Tokenizer::for_model("gpt-6.1-sol"), Some(Tokenizer::O200k));
        assert_eq!(Tokenizer::for_model("gpt-5.6-luna"), Some(Tokenizer::O200k));
        assert_eq!(Tokenizer::for_model("gpt-4o"), None);
        assert_eq!(Tokenizer::for_model("deepseek-v4.1-flash"), None);
    }

    #[test]
    fn anthropic_requests_split_by_part() {
        let payload = json!({
            "model": "claude-sonnet-5-5",
            "system": [{"type": "text", "text": "Be brief.", "cache_control": {"type": "ephemeral"}}],
            "tools": [{"name": "read", "description": "Read a file", "input_schema": {"type": "object"}}],
            "messages": [
                {"role": "user", "content": "hello"},
                {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": "plan", "signature": "sig"},
                    {"type": "tool_use", "id": "t1", "name": "read", "input": {"path": "a.rs"}}
                ]},
                {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1", "content": "fn main() {}"}]}
            ]
        });
        let segments = extract("/v1/messages", &payload).unwrap();
        assert!(segments.system);
        assert_eq!(
            (segments.messages, segments.blocks, segments.calls),
            (3, 4, 2)
        );
        let shape = segments.count(Tokenizer::O200k);
        let o200k = |t: &str| tiktoken_rs::o200k_base_singleton().encode_ordinary(t).len() as u64;
        assert_eq!(shape.text, o200k("Be brief.") + o200k("hello"));
        assert_eq!(shape.thinking, o200k("plan"));
        assert_eq!(
            shape.tool,
            o200k("read") + o200k(r#"{"path":"a.rs"}"#) + o200k("fn main() {}")
        );
        assert_eq!(
            shape.definitions,
            o200k("// Read a file\ntype read = () => any;\n\n")
        );
        let tools = &payload["tools"][0].to_string();
        assert_eq!(
            segments.count(Tokenizer::ClaudeV48).definitions,
            Tokenizer::ClaudeV48.count(tools)
        );
    }

    #[test]
    fn openai_tools_render_as_typescript() {
        let tool = json!({"type": "function", "function": {
            "name": "weather",
            "description": "Current weather.",
            "parameters": {"type": "object", "required": ["city"], "properties": {
                "city": {"type": "string", "description": "City name"},
                "unit": {"type": "string", "enum": ["c", "f"]},
                "days": {"type": "array", "items": {"type": "integer"}}
            }}
        }});
        assert_eq!(
            typescript_tool(&tool).unwrap(),
            "// Current weather.\ntype weather = (_: {\n// City name\ncity: string,\n\
             days?: number[],\nunit?: \"c\" | \"f\",\n}) => any;\n\n"
        );
        assert!(typescript_tool(&json!({"type": "web_search"})).is_none());
    }

    #[test]
    fn openai_requests_split_by_part() {
        let chat = json!({
            "messages": [
                {"role": "system", "content": "sys"},
                {"role": "assistant", "content": null, "tool_calls": [{"id": "c", "type": "function", "function": {"name": "f", "arguments": "{\"a\":1}"}}]},
                {"role": "tool", "tool_call_id": "c", "content": "42"}
            ],
            "tools": [{"type": "function", "function": {"name": "f", "parameters": {}}}]
        });
        let segments = extract("/v1/chat/completions", &chat).unwrap();
        assert!(segments.system);
        assert_eq!(
            (segments.messages, segments.blocks, segments.calls),
            (0, 3, 2)
        );
        let responses = json!({
            "instructions": "sys",
            "input": [
                {"role": "user", "content": [{"type": "input_text", "text": "hi"}]},
                {"type": "reasoning", "summary": [{"type": "summary_text", "text": "think"}]},
                {"type": "function_call", "call_id": "c", "name": "shell", "arguments": "{}"},
                {"type": "function_call_output", "call_id": "c", "output": "ok"}
            ]
        });
        let segments = extract("/v1/responses", &responses).unwrap();
        assert_eq!(
            (segments.messages, segments.blocks, segments.calls),
            (1, 4, 2)
        );
        assert!(segments.count(Tokenizer::O200k).thinking > 0);
    }

    #[test]
    fn frames_reproduce_official_counts() {
        // Official counts recorded during calibration.
        let shape = |tokenizer, text, system, messages| json!({"tokenizer": tokenizer, "text": text, "system": system, "messages": messages});
        let expected = |shape: &Value, model| expected_tokens(shape, model).unwrap();
        assert_eq!(
            expected(&shape("claude-v4.8", 2, false, 1), "claude-sonnet-5-5"),
            10.0
        );
        assert_eq!(
            expected(&shape("claude-v4.8", 2, false, 1), "claude-opus-4-8"),
            8.0
        );
        assert_eq!(
            expected(&shape("claude-v4.8", 392, true, 5), "claude-opus-5-5"),
            409.0
        );
        assert_eq!(expected(&shape("o200k", 1, false, 1), "gpt-6.1-sol"), 7.0);
        assert_eq!(expected(&shape("o200k", 58, true, 1), "gpt-6.1-sol"), 68.0);
        assert!(expected_tokens(&shape("cl100k", 1, false, 1), "gpt-4").is_none());
    }

    #[test]
    fn only_calibrated_models_on_native_apis_are_counted() {
        for model in [
            "claude-sonnet-5-5",
            "anthropic/claude-sonnet-5.5",
            "Claude-Opus-4-8-20260815",
            "gpt-6.1-sol",
        ] {
            assert!(calibrated(model), "{model}");
        }
        for model in [
            "claude-sonnet-5-6",
            "claude-opus-6",
            "gpt-6-luna",
            "cc-sonnet",
        ] {
            assert!(!calibrated(model), "{model}");
        }
        let messages = json!({"messages": [{"role": "user", "content": "hi"}]});
        assert!(request_tokens("/v1/messages", &messages, "claude-sonnet-5-5").is_ok());
        // Chat requests to Claude are converted by the relay, which is not calibrated.
        assert_eq!(
            request_tokens("/v1/chat/completions", &messages, "claude-sonnet-5-5"),
            Err("converted_protocol")
        );
        assert!(request_tokens("/v1/chat/completions", &messages, "gpt-6.1-sol").is_ok());
        assert_eq!(
            request_tokens("/v1/messages", &messages, "gpt-6.1-sol"),
            Err("converted_protocol")
        );
        assert_eq!(
            request_tokens("/v1/chat/completions", &messages, "gpt-6-luna"),
            Err("uncalibrated")
        );
    }

    #[test]
    fn uncountable_content_is_not_estimated() {
        let image = json!({"messages": [{"role": "user", "content": [
            {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "AA"}}
        ]}]});
        assert!(extract("/v1/messages", &image).is_none());
        let redacted = json!({"messages": [{"role": "assistant", "content": [{"type": "redacted_thinking", "data": "x"}]}]});
        assert!(extract("/v1/messages", &redacted).is_none());
        let encrypted =
            json!({"input": [{"type": "reasoning", "summary": [], "encrypted_content": "gAAA"}]});
        assert!(extract("/v1/responses", &encrypted).is_none());
        let image_url = json!({"messages": [{"role": "user", "content": [{"type": "image_url", "image_url": {"url": "x"}}]}]});
        assert!(extract("/v1/chat/completions", &image_url).is_none());
        assert!(extract("/v1beta/models/gemini:generateContent", &json!({})).is_none());
    }
}

#[cfg(test)]
mod calibration {
    use super::*;
    use serde_json::json;

    /// Prints the local shape of each recorded request next to its reported
    /// tokens, for fitting frames against official counts. Run with
    /// CALIBRATION=<jsonl of {path, model, payload, reported}>.
    #[test]
    #[ignore = "needs CALIBRATION pointing at recorded official counts"]
    fn calibration_shapes() {
        let file = std::env::var("CALIBRATION").unwrap();
        for line in std::fs::read_to_string(file).unwrap().lines() {
            let row: Value = serde_json::from_str(line).unwrap();
            let model = row["model"].as_str().unwrap();
            let model = model.rsplit('/').next().unwrap().replace('.', "-");
            let tokenizer = Tokenizer::for_model(&model).unwrap();
            let shape =
                extract(row["path"].as_str().unwrap(), &row["payload"]).map(|s| s.count(tokenizer));
            println!(
                "SHAPE {}",
                json!({"model": model, "reported": row["reported"], "shape": shape})
            );
        }
    }
}
