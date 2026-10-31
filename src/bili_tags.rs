//! The bili context-compression proxy prefixes history text with reference
//! tags such as `<acp tokens="2.1K" type="text">m00379</acp>`. On an assistant
//! turn the tag keeps the history from matching what the model said, so
//! upstreams that resume a conversation by its exact text (magpie) restart it
//! and rewrite the whole prompt cache every turn. On a tool-result turn it
//! hides the leading `<total_tokens>` notice, so the notice reads as a user
//! interjection. Removing just those leading tags is deterministic, so prompt
//! caching on other upstreams is unaffected. bili looks references up in its
//! own state, so compression keeps working.
//!
//! bili tags every text block, and a turn can hold several: when it runs one
//! of its own tools mid-reply, the text before and after the tool call comes
//! back as separate blocks of one assistant turn. Each of those leading tags
//! is removed, not only the first.
//!
//! bili tags the model's turns the same way in every protocol. It leaves
//! OpenAI and Gemini tool results untagged, so only Anthropic has a second
//! kind of turn to clean.
use serde_json::Value;

/// Where a protocol keeps the conversation.
struct Format {
    /// The array of turns.
    turns: &'static str,
    /// The role of the model's own turns.
    model: &'static str,
    /// The field holding a turn's content.
    content: &'static str,
    /// The type of text blocks; Gemini parts have no type.
    text: Option<&'static str>,
}

/// Anthropic Messages and OpenAI Chat Completions.
const MESSAGES: Format = Format {
    turns: "messages",
    model: "assistant",
    content: "content",
    text: Some("text"),
};
const RESPONSES: Format = Format {
    turns: "input",
    model: "assistant",
    content: "content",
    text: Some("output_text"),
};
const GEMINI: Format = Format {
    turns: "contents",
    model: "model",
    content: "parts",
    text: None,
};

/// Removes the tag that starts each text block of the model's turns and of
/// Anthropic user turns carrying tool results, for the gateway request at
/// `path`. Returns how many were removed.
pub fn strip(path: &str, payload: &mut Value) -> u32 {
    let format = match path {
        "/v1/messages" | "/v1/chat/completions" => &MESSAGES,
        "/v1/responses" => &RESPONSES,
        _ if path.starts_with("/v1beta/") => &GEMINI,
        _ => return 0,
    };
    let Some(turns) = payload.get_mut(format.turns).and_then(Value::as_array_mut) else {
        return 0;
    };
    let mut stripped = 0;
    for message in turns {
        let Some(message) = message.as_object_mut() else {
            continue;
        };
        // Responses input also holds tool calls and their outputs.
        if message
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind != "message")
        {
            continue;
        }
        let role = message.get("role").and_then(Value::as_str);
        let has_results = message
            .get("content")
            .and_then(Value::as_array)
            .is_some_and(|blocks| {
                blocks
                    .iter()
                    .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
            });
        if role != Some(format.model) && !(role == Some("user") && has_results) {
            continue;
        }
        stripped += strip_content(message.get_mut(format.content), format.text);
    }
    stripped
}

fn strip_content(content: Option<&mut Value>, kind: Option<&str>) -> u32 {
    match content {
        Some(Value::String(text)) => match tag_len(text) {
            // A message cannot be left empty, so a tag-only string stays.
            Some(len) if len < text.len() => {
                text.drain(..len);
                1
            }
            _ => 0,
        },
        Some(Value::Array(blocks)) => {
            let mut stripped = 0;
            let mut index = 0;
            while index < blocks.len() {
                let block = &blocks[index];
                let is_text = match kind {
                    Some(kind) => block.get("type").and_then(Value::as_str) == Some(kind),
                    None => true,
                };
                let tagged = block
                    .get("text")
                    .and_then(Value::as_str)
                    .filter(|_| is_text)
                    .and_then(|text| Some((tag_len(text)?, text.len())));
                let Some((len, text_len)) = tagged else {
                    index += 1;
                    continue;
                };
                if len < text_len {
                    let rest = blocks[index]["text"].as_str().unwrap_or_default()[len..].to_owned();
                    blocks[index]["text"] = Value::String(rest);
                    stripped += 1;
                    index += 1;
                    continue;
                }
                // Empty text blocks are rejected upstream, so drop the block,
                // keeping its cache breakpoint on a neighbour. The last block
                // stays, since the turn cannot be left empty.
                if blocks.len() == 1 {
                    break;
                }
                let removed = blocks.remove(index);
                if let Some(cache_control) = removed.get("cache_control") {
                    let neighbour = index.saturating_sub(1);
                    if let Some(block) = blocks[neighbour].as_object_mut() {
                        block
                            .entry("cache_control")
                            .or_insert_with(|| cache_control.clone());
                    }
                }
                stripped += 1;
            }
            stripped
        }
        _ => 0,
    }
}

/// Length of a leading `<acp tokens=".." type="..">mDIGITS</acp>` tag and the
/// newline after it, if `text` starts with one.
fn tag_len(text: &str) -> Option<usize> {
    let mut rest = text.strip_prefix("<acp tokens=\"")?;
    rest = skip_attribute(rest)?.strip_prefix(" type=\"")?;
    rest = skip_attribute(rest)?.strip_prefix(">m")?;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    rest = rest[digits..].strip_prefix("</acp>")?;
    rest = rest.strip_prefix('\n').unwrap_or(rest);
    Some(text.len() - rest.len())
}

/// Skips an attribute value up to and including its closing quote.
fn skip_attribute(text: &str) -> Option<&str> {
    let end = text.find(['"', '<', '>'])?;
    text[end..].strip_prefix('"')
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TAG: &str = "<acp tokens=\"2.1K\" type=\"text\">m00379</acp>\n";

    #[test]
    fn strips_leading_tags_from_assistant_and_tool_result_turns() {
        let mut payload = json!({"messages": [
            {"role": "user", "content": format!("{TAG}hello")},
            {"role": "assistant", "content": [
                {"type": "text", "text": format!("{TAG}answer {TAG}")},
                {"type": "tool_use", "id": "t1", "name": "x", "input": {}},
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "t1", "content": "ok"},
                {"type": "text", "text": format!("{TAG}<total_tokens>9</total_tokens>")},
            ]},
            {"role": "assistant", "content": format!("{TAG}done")},
        ]});
        assert_eq!(strip("/v1/messages", &mut payload), 3);
        let messages = &payload["messages"];
        // Plain user turns keep their tag; it is the same every turn.
        assert_eq!(messages[0]["content"], format!("{TAG}hello"));
        // Only the leading tag goes.
        assert_eq!(messages[1]["content"][0]["text"], format!("answer {TAG}"));
        assert_eq!(
            messages[2]["content"][1]["text"],
            "<total_tokens>9</total_tokens>"
        );
        assert_eq!(messages[3]["content"], "done");
        let once = payload.clone();
        assert_eq!(strip("/v1/messages", &mut payload), 0);
        assert_eq!(payload, once);
    }

    #[test]
    fn tag_only_blocks_are_dropped_keeping_their_cache_breakpoint() {
        let mut payload = json!({"messages": [
            {"role": "assistant", "content": [
                {"type": "tool_use", "id": "t1", "name": "x", "input": {}},
                {"type": "text", "text": TAG, "cache_control": {"type": "ephemeral"}},
            ]},
            {"role": "assistant", "content": [{"type": "text", "text": TAG}]},
            {"role": "assistant", "content": TAG.trim_end()},
        ]});
        assert_eq!(strip("/v1/messages", &mut payload), 1);
        let messages = &payload["messages"];
        assert_eq!(
            messages[0]["content"],
            json!([{"type": "tool_use", "id": "t1", "name": "x", "input": {},
                    "cache_control": {"type": "ephemeral"}}])
        );
        assert_eq!(messages[1]["content"][0]["text"], TAG);
        assert_eq!(messages[2]["content"], TAG.trim_end());
    }

    #[test]
    fn strips_every_text_block_of_a_turn() {
        // bili's own tool calls split one reply into several text blocks.
        let mut payload = json!({"messages": [
            {"role": "assistant", "content": [
                {"type": "text", "text": format!("{TAG}before")},
                {"type": "text", "text": format!("{TAG}after {TAG}")},
                {"type": "text", "text": TAG, "cache_control": {"type": "ephemeral"}},
                {"type": "tool_use", "id": "t1", "name": "x", "input": {}},
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "t1", "content": "ok"},
                {"type": "text", "text": format!("{TAG}<total_tokens>9</total_tokens>")},
                {"type": "text", "text": format!("{TAG}<system-reminder>x</system-reminder>")},
            ]},
            {"role": "assistant", "content": [
                {"type": "text", "text": TAG},
                {"type": "text", "text": TAG},
            ]},
        ]});
        assert_eq!(strip("/v1/messages", &mut payload), 6);
        let messages = &payload["messages"];
        assert_eq!(
            messages[0]["content"],
            json!([
                {"type": "text", "text": "before"},
                {"type": "text", "text": format!("after {TAG}"),
                 "cache_control": {"type": "ephemeral"}},
                {"type": "tool_use", "id": "t1", "name": "x", "input": {}},
            ])
        );
        assert_eq!(
            messages[1]["content"][1]["text"],
            "<total_tokens>9</total_tokens>"
        );
        assert_eq!(
            messages[1]["content"][2]["text"],
            "<system-reminder>x</system-reminder>"
        );
        // One tag-only block is kept so the turn is not empty.
        assert_eq!(
            messages[2]["content"],
            json!([{"type": "text", "text": TAG}])
        );
        let once = payload.clone();
        assert_eq!(strip("/v1/messages", &mut payload), 0);
        assert_eq!(payload, once);
    }

    #[test]
    fn strips_model_turns_in_openai_and_gemini_requests() {
        let mut chat = json!({"messages": [
            {"role": "user", "content": format!("{TAG}hi")},
            {"role": "assistant", "content": format!("{TAG}hello"), "tool_calls": []},
            {"role": "tool", "tool_call_id": "t1", "content": format!("{TAG}done")},
            {"role": "assistant", "content": [{"type": "text", "text": format!("{TAG}more")}]},
        ]});
        assert_eq!(strip("/v1/chat/completions", &mut chat), 2);
        assert_eq!(chat["messages"][0]["content"], format!("{TAG}hi"));
        assert_eq!(chat["messages"][1]["content"], "hello");
        assert_eq!(chat["messages"][2]["content"], format!("{TAG}done"));
        assert_eq!(chat["messages"][3]["content"][0]["text"], "more");

        let mut responses = json!({"input": [
            {"type": "message", "role": "user", "content": [{"type": "input_text", "text": format!("{TAG}hi")}]},
            {"type": "message", "role": "assistant", "content": [
                {"type": "refusal", "refusal": "no"},
                {"type": "output_text", "text": format!("{TAG}hello")},
            ]},
            {"role": "assistant", "content": format!("{TAG}short form")},
            {"type": "function_call_output", "call_id": "c1", "role": "assistant", "output": "done"},
        ]});
        assert_eq!(strip("/v1/responses", &mut responses), 2);
        let input = &responses["input"];
        assert_eq!(input[0]["content"][0]["text"], format!("{TAG}hi"));
        assert_eq!(input[1]["content"][1]["text"], "hello");
        assert_eq!(input[2]["content"], "short form");
        assert_eq!(strip("/v1/responses", &mut json!({"input": "hi"})), 0);

        let mut gemini = json!({"contents": [
            {"role": "user", "parts": [{"text": format!("{TAG}hi")}]},
            {"role": "model", "parts": [
                {"functionCall": {"name": "x", "args": {}}},
                {"text": format!("{TAG}hello")},
            ]},
            {"role": "model", "parts": [{"text": TAG}, {"functionCall": {"name": "x", "args": {}}}]},
        ]});
        let path = "/v1beta/models/gemini-x:streamGenerateContent";
        assert_eq!(strip(path, &mut gemini), 2);
        let contents = &gemini["contents"];
        assert_eq!(contents[0]["parts"][0]["text"], format!("{TAG}hi"));
        assert_eq!(contents[1]["parts"][1]["text"], "hello");
        assert_eq!(
            contents[2]["parts"],
            json!([{"functionCall": {"name": "x", "args": {}}}])
        );
    }

    #[test]
    fn leaves_other_text_untouched() {
        for text in [
            "<acp tokens=\"1\" type=\"text\">x1</acp>\nhi",
            "<acp tokens=\"1\" type=\"text\">m</acp>\nhi",
            "<acp tokens=\"1\" type=\"text\">m1</acp hi",
            "<acp tokens=\"1>\" type=\"text\">m1</acp>\nhi",
            " <acp tokens=\"1\" type=\"text\">m1</acp>\nhi",
            "plain",
        ] {
            let mut payload = json!({"messages": [{"role": "assistant", "content": text}]});
            assert_eq!(strip("/v1/messages", &mut payload), 0, "{text}");
            assert_eq!(payload["messages"][0]["content"], text);
        }
        assert_eq!(strip("/v1/messages", &mut json!({"contents": []})), 0);
        let mut payload =
            json!({"messages": [{"role": "assistant", "content": format!("{TAG}x")}]});
        assert_eq!(strip("/v1/embeddings", &mut payload), 0);
        assert_eq!(
            tag_len("<acp tokens=\"\" type=\"summary\">m12</acp>x"),
            Some(39)
        );
    }
}
