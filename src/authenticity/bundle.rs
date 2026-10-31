//! Baseline import bundles (format v1).
//!
//! A bundle is one JSON Lines file: the first line is the manifest, every
//! following line is a sample. `samples_sha256` covers the exact bytes after
//! the first newline. Unknown fields are rejected, so answer text, keys or
//! proxy details can never slip into an import.
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const FORMAT: &str = "vair-baseline";
pub const FORMAT_VERSION: i32 = 1;
pub const MAX_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_SAMPLES: usize = 5000;

#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: String,
    pub format_version: i32,
    pub batch_id: Uuid,
    pub plan_id: Uuid,
    pub model: String,
    pub source: String,
    pub openrouter_model: String,
    pub provider_only: String,
    pub pool_version: i32,
    pub reasoning: String,
    pub range_max: i32,
    pub tool_version: String,
    pub started_at: String,
    pub finished_at: String,
    pub sample_count: usize,
    pub samples_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sample {
    pub sample_id: String,
    pub probe_kind: String,
    pub template_id: Uuid,
    pub n: Option<i32>,
    #[serde(default)]
    pub values: Vec<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
    pub served_provider: Option<String>,
    /// Accepted for the collector's local diagnostics; not stored.
    #[allow(dead_code)]
    pub finish_reason: Option<String>,
}

#[derive(Debug)]
pub struct Bundle {
    pub manifest: Manifest,
    pub samples: Vec<Sample>,
    pub sha256: String,
    /// Samples dropped because their `sample_id` repeated an earlier one.
    pub duplicates: usize,
}

pub fn parse(bytes: &[u8]) -> Result<Bundle, String> {
    if bytes.len() > MAX_BYTES {
        return Err("导入包超过 20 MB".into());
    }
    let split = bytes
        .iter()
        .position(|&b| b == b'\n')
        .ok_or("导入包缺少样本")?;
    let (head, body) = (&bytes[..split], &bytes[split + 1..]);
    let manifest: Manifest = serde_json::from_slice(head)
        .map_err(|e| format!("清单格式无效：{}", short(&e.to_string())))?;
    if manifest.format != FORMAT || manifest.format_version != FORMAT_VERSION {
        return Err(format!(
            "不支持的导入包格式 {} v{}",
            manifest.format, manifest.format_version
        ));
    }
    let sha256 = hex::encode(Sha256::digest(body));
    if !sha256.eq_ignore_ascii_case(&manifest.samples_sha256) {
        return Err("样本校验和与清单不一致，导入包可能已损坏或被修改".into());
    }
    let text = std::str::from_utf8(body).map_err(|_| "样本不是 UTF-8 文本")?;
    let mut samples = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut duplicates = 0;
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let sample: Sample = serde_json::from_str(line)
            .map_err(|e| format!("第 {} 个样本格式无效：{}", index + 1, short(&e.to_string())))?;
        if !matches!(sample.probe_kind.as_str(), "fingerprint" | "token") {
            return Err(format!("第 {} 个样本的探针类型无效", index + 1));
        }
        if sample.sample_id.is_empty() || sample.sample_id.len() > 64 {
            return Err(format!("第 {} 个样本的 sample_id 无效", index + 1));
        }
        if !seen.insert(sample.sample_id.clone()) {
            duplicates += 1;
            continue;
        }
        samples.push(sample);
        if samples.len() > MAX_SAMPLES {
            return Err(format!("单个导入包最多 {MAX_SAMPLES} 个样本"));
        }
    }
    if samples.len() + duplicates != manifest.sample_count {
        return Err(format!(
            "清单声明 {} 个样本，实际 {} 个",
            manifest.sample_count,
            samples.len() + duplicates
        ));
    }
    Ok(Bundle {
        manifest,
        samples,
        sha256,
        duplicates,
    })
}

fn short(message: &str) -> String {
    message.chars().take(160).collect()
}

/// Provider names differ in spelling between pins and responses
/// ("google-ai-studio" vs "Google AI Studio"), so compare letters and digits.
pub fn same_provider(pin: &str, served: &str) -> bool {
    let norm = |s: &str| {
        s.chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_lowercase())
            .collect::<String>()
    };
    let (pin, served) = (norm(pin), norm(served));
    !pin.is_empty() && (served == pin || served.starts_with(&pin) || pin.starts_with(&served))
}

#[cfg(test)]
pub fn build(manifest: serde_json::Value, samples: &[serde_json::Value]) -> Vec<u8> {
    let body: String = samples.iter().map(|s| format!("{s}\n")).collect();
    let mut manifest = manifest;
    manifest["sample_count"] = samples.len().into();
    manifest["samples_sha256"] = hex::encode(Sha256::digest(body.as_bytes())).into();
    format!("{manifest}\n{body}").into_bytes()
}

#[cfg(test)]
pub fn test_manifest(plan_id: Uuid, model: &str, pool_version: i32) -> serde_json::Value {
    serde_json::json!({
        "format": FORMAT, "format_version": FORMAT_VERSION, "batch_id": Uuid::new_v4(),
        "plan_id": plan_id, "model": model, "source": "openrouter", "openrouter_model": format!("vendor/{model}"),
        "provider_only": "acme", "pool_version": pool_version, "reasoning": "off", "range_max": 355,
        "tool_version": "test", "started_at": "2026-10-09T00:00:00Z", "finished_at": "2026-10-09T01:00:00Z",
        "sample_count": 0, "samples_sha256": ""
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample(id: &str) -> serde_json::Value {
        json!({"sample_id": id, "probe_kind": "fingerprint", "template_id": Uuid::nil(), "n": 3,
            "values": [1, 2, 3], "input_tokens": 10, "output_tokens": 5, "reasoning_tokens": 0,
            "served_provider": "Acme", "finish_reason": "stop"})
    }

    #[test]
    fn parses_and_deduplicates() {
        let bytes = build(
            test_manifest(Uuid::new_v4(), "m", 1),
            &[sample("a"), sample("b"), sample("a")],
        );
        let bundle = parse(&bytes).unwrap();
        assert_eq!(bundle.samples.len(), 2);
        assert_eq!(bundle.duplicates, 1);
    }

    #[test]
    fn rejects_tampering_unknown_fields_and_wrong_format() {
        let mut bytes = build(test_manifest(Uuid::new_v4(), "m", 1), &[sample("a")]);
        let last = bytes.len() - 3;
        bytes[last] = b'9';
        assert!(parse(&bytes).unwrap_err().contains("校验和"));

        let mut leaky = sample("a");
        leaky["content"] = json!("the raw answer");
        assert!(parse(&build(test_manifest(Uuid::new_v4(), "m", 1), &[leaky])).is_err());

        let mut manifest = test_manifest(Uuid::new_v4(), "m", 1);
        manifest["api_key"] = json!("sk-should-never-be-here");
        assert!(parse(&build(manifest, &[sample("a")])).is_err());

        let mut manifest = test_manifest(Uuid::new_v4(), "m", 1);
        manifest["format_version"] = json!(2);
        assert!(parse(&build(manifest, &[sample("a")])).is_err());
        assert!(parse(b"{}").is_err());
    }

    #[test]
    fn provider_names_compare_loosely() {
        assert!(same_provider("google-ai-studio", "Google AI Studio"));
        assert!(same_provider("anthropic", "Anthropic"));
        assert!(!same_provider("anthropic", "Amazon Bedrock"));
        assert!(!same_provider("", "Anthropic"));
    }
}
