//! A connectivity check that costs nothing: list models with the key, the way
//! the agent's protocol would, and look for the chosen model there. The list
//! only covers names the gateway knows, so a missing model is not a failure.
use crate::agents::Protocol;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Instant;
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub ok: bool,
    pub message: String,
    pub latency_ms: u64,
    pub checked_at: DateTime<Utc>,
}

pub async fn test(server: &Url, protocol: Protocol, key: &str, model: &str) -> TestResult {
    let started = Instant::now();
    let root = server.as_str().trim_end_matches('/');
    let http = reqwest::Client::new();
    let request = match protocol {
        Protocol::OpenAi => http.get(format!("{root}/v1/models")).bearer_auth(key),
        Protocol::Anthropic => http
            .get(format!("{root}/v1/models"))
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01"),
        Protocol::Gemini => http
            .get(format!("{root}/v1beta/models"))
            .header("x-goog-api-key", key),
    };
    let outcome = request
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await;
    let latency_ms = started.elapsed().as_millis() as u64;
    let (ok, message) = match outcome {
        Err(error) => (false, format!("无法连接网关：{}", error.without_url())),
        Ok(response) if !response.status().is_success() => {
            let status = response.status();
            let body: serde_json::Value = response.json().await.unwrap_or_default();
            (false, gateway_reason(status, &body))
        }
        Ok(response) => (
            true,
            listed_message(&response.text().await.unwrap_or_default(), model),
        ),
    };
    TestResult {
        ok,
        message,
        latency_ms,
        checked_at: Utc::now(),
    }
}

fn listed_message(body: &str, model: &str) -> String {
    if body.contains(&format!("\"{model}\"")) || body.contains(&format!("models/{model}\"")) {
        "连接正常".into()
    } else {
        format!("连接正常；{model} 不在网关模型列表中，能否使用由上游决定")
    }
}

/// The gateway names the rule that refused a key (paused, over a limit, …);
/// the link to the web page that follows it is dropped for the short status.
fn gateway_reason(status: reqwest::StatusCode, body: &serde_json::Value) -> String {
    if let Some(message) = body["error"]["message"].as_str() {
        return message.split("。在 ").next().unwrap_or(message).to_owned();
    }
    if status == reqwest::StatusCode::UNAUTHORIZED {
        "Key 无效、已停用或已撤销".into()
    } else {
        format!("网关返回 {}", status.as_u16())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unlisted_models_still_pass() {
        assert_eq!(
            listed_message(r#"{"data":[{"id":"kimi-k3"}]}"#, "kimi-k3"),
            "连接正常"
        );
        assert_eq!(
            listed_message(r#"{"models":[{"name":"models/g-1"}]}"#, "g-1"),
            "连接正常"
        );
        assert!(listed_message(r#"{"data":[]}"#, "glm-5.3").contains("由上游决定"));
    }

    #[test]
    fn gateway_reasons_drop_the_help_link() {
        let body = json!({"error": {"code": "key_paused", "message": "此 OpenVAIR Key 已暂停，请联系 Key 的发放方。在 https://openvair.example.com/my-key 查看此 Key 的状态与额度"}});
        assert_eq!(
            gateway_reason(reqwest::StatusCode::UNAUTHORIZED, &body),
            "此 OpenVAIR Key 已暂停，请联系 Key 的发放方"
        );
        assert_eq!(
            gateway_reason(reqwest::StatusCode::UNAUTHORIZED, &json!(null)),
            "Key 无效、已停用或已撤销"
        );
        assert_eq!(
            gateway_reason(reqwest::StatusCode::BAD_GATEWAY, &json!(null)),
            "网关返回 502"
        );
    }
}
