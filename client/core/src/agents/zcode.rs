//! ZCode (3.14 and later) reads `~/.zcode/v2/provider_config.json` and
//! watches it, so changes apply without a restart. OpenVAIR adds an `openvair`
//! provider rule listing the key's models, puts it first in the provider
//! order and selects the chosen model; other providers are left alone.
//!
//! ZCode checks the file strictly and rejects keys it does not know, so OpenVAIR
//! writes only the fields ZCode itself writes, and only into a file ZCode has
//! created: an unknown `schemaVersion` or a data folder moved elsewhere is
//! left to the user.
use super::{
    Current, Meta, Prior, Protocol, Settings, listed_models, lookup, object_mut, read_json,
    remember, text, write_json,
};
use crate::{CoreError, Result};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

const PROVIDER: &str = "openvair";
const SCHEMA: u64 = 1;
const SELECTION: &str = "config.defaultModelSelection";
const FILE: &str = "v2/provider_config.json";

fn api_type(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::Anthropic => "anthropic-messages",
        _ => "openai-chat-completions",
    }
}

/// The data folder ZCode was told to use instead of `~/.zcode`, if any.
fn moved(dir: &Path) -> Option<String> {
    let settings = read_json(&dir.join("v2/setting.json")).ok()?;
    text(settings.get("dataBaseDir")).filter(|folder| !folder.trim().is_empty())
}

fn rules(config: &Map<String, Value>) -> Option<&Vec<Value>> {
    lookup(config, &["config", "providerConfigRules", "providerRules"])?.as_array()
}

fn is_ours(rule: &Value) -> bool {
    rule.get("providerId").and_then(Value::as_str) == Some(PROVIDER)
}

fn our_rule(config: &Map<String, Value>) -> Option<&Value> {
    rules(config)?.iter().find(|rule| is_ours(rule))
}

fn our_rule_mut(config: &mut Map<String, Value>) -> Option<&mut Map<String, Value>> {
    config
        .get_mut("config")?
        .get_mut("providerConfigRules")?
        .get_mut("providerRules")?
        .as_array_mut()?
        .iter_mut()
        .find(|rule| is_ours(rule))?
        .as_object_mut()
}

fn selected(config: &Map<String, Value>) -> Option<String> {
    let selection = lookup(config, &["config", "defaultModelSelection"])?;
    (selection.get("providerId").and_then(Value::as_str) == Some(PROVIDER))
        .then(|| text(selection.get("modelId")))
        .flatten()
}

/// Loads the file ZCode created, refusing anything OpenVAIR cannot safely edit.
fn load(path: &Path) -> Result<Map<String, Value>> {
    if let Some(dir) = path.parent().and_then(Path::parent)
        && let Some(folder) = moved(dir)
    {
        return Err(CoreError::Other(format!(
            "ZCode 的数据目录改到了 {folder}，请在“接入 Agent”里选“手动接入”"
        )));
    }
    if !path.exists() {
        return Err(CoreError::Other(
            "没有找到 ZCode 的服务商配置，请先打开一次 ZCode（3.14 或更新版本）".into(),
        ));
    }
    let config = read_json(path)?;
    if config.get("schemaVersion").and_then(Value::as_u64) != Some(SCHEMA) {
        return Err(CoreError::Other(
            "这个版本的 ZCode 配置格式 OpenVAIR 还不认识，请在“接入 Agent”里选“手动接入”".into(),
        ));
    }
    Ok(config)
}

fn models(model: &str, models: &[String]) -> Value {
    json!(listed_models(model, models))
}

pub fn read(files: &[PathBuf]) -> Result<Current> {
    let config = read_json(&files[0])?;
    let Some(rule) = our_rule(&config) else {
        return Ok(Current::default());
    };
    Ok(Current {
        base_url: text(rule.pointer("/config/api/baseUrl")),
        secret: text(rule.pointer("/config/access/apiKey")),
        model: selected(&config),
    })
}

pub fn apply(files: &[PathBuf], settings: &Settings) -> Result<Prior> {
    let path = &files[0];
    let mut config = load(path)?;
    let mut prior = Prior::new();
    remember(
        &mut prior,
        SELECTION,
        lookup(&config, &["config", "defaultModelSelection"]).cloned(),
    );
    let protocol = settings.protocol.unwrap_or(Protocol::OpenAi);
    let list = models(settings.model, settings.models);
    let rule = json!({
        "providerId": PROVIDER,
        "providerName": "OpenVAIR",
        "enabled": true,
        "config": {
            "group": "standard-personal",
            "access": { "type": "api-key", "apiKey": settings.secret },
            "api": {
                "type": api_type(protocol),
                "baseUrl": super::protocol_url(protocol, settings.server),
                "headers": null,
            },
            "personalModelIds": list,
            "modelOrder": list,
        },
    });
    let body = object_mut(&mut config, "config");
    let order = body
        .entry("providerOrder")
        .or_insert_with(|| Value::Array(Vec::new()));
    if !order.is_array() {
        *order = Value::Array(Vec::new());
    }
    let order = order.as_array_mut().expect("an array");
    order.retain(|id| id.as_str() != Some(PROVIDER));
    order.insert(0, json!(PROVIDER));
    let rules = object_mut(body, "providerConfigRules")
        .entry("providerRules")
        .or_insert_with(|| Value::Array(Vec::new()));
    if !rules.is_array() {
        *rules = Value::Array(Vec::new());
    }
    let rules = rules.as_array_mut().expect("an array");
    rules.retain(|rule| !is_ours(rule));
    rules.push(rule);
    body.insert(
        "defaultModelSelection".into(),
        json!({ "providerId": PROVIDER, "modelId": settings.model }),
    );
    write_json(path, config)?;
    Ok(prior)
}

pub fn sync_models(files: &[PathBuf], model: &str, list: &[String]) -> Result<bool> {
    let path = &files[0];
    let mut config = read_json(path)?;
    let next = models(model, list);
    let Some(rule) = our_rule_mut(&mut config).and_then(|rule| rule.get_mut("config")) else {
        return Ok(false);
    };
    if rule.get("personalModelIds") == Some(&next) {
        return Ok(false);
    }
    let Some(rule) = rule.as_object_mut() else {
        return Ok(false);
    };
    rule.insert("personalModelIds".into(), next.clone());
    rule.insert("modelOrder".into(), next);
    write_json(path, config)?;
    Ok(true)
}

pub fn remove(files: &[PathBuf], prior: &Prior) -> Result<()> {
    let path = &files[0];
    if !path.exists() {
        return Ok(());
    }
    let mut config = read_json(path)?;
    let ours = selected(&config).is_some();
    let Some(body) = config.get_mut("config").and_then(Value::as_object_mut) else {
        return Ok(());
    };
    if let Some(order) = body.get_mut("providerOrder").and_then(Value::as_array_mut) {
        order.retain(|id| id.as_str() != Some(PROVIDER));
    }
    if let Some(rules) = body
        .get_mut("providerConfigRules")
        .and_then(|rules| rules.get_mut("providerRules"))
        .and_then(Value::as_array_mut)
    {
        rules.retain(|rule| !is_ours(rule));
    }
    if ours {
        match prior.get(SELECTION).cloned().flatten() {
            Some(old) => {
                body.insert("defaultModelSelection".into(), old);
            }
            None => {
                body.remove("defaultModelSelection");
            }
        }
    }
    write_json(path, config)
}

pub struct ZCode;

impl super::Adapter for ZCode {
    fn meta(&self) -> &'static Meta {
        &Meta {
            name: "ZCode",
            glyph: "Z",
            homepage: "https://zcode.z.ai/",
            protocols: &[Protocol::OpenAi, Protocol::Anthropic],
            binaries: &[],
            traces: &[".zcode/v2/logs", "/Applications/ZCode.app"],
            dir: ".zcode",
            files: &[FILE],
            fields: [
                "providerRules[openvair].api.baseUrl",
                "providerRules[openvair].access.apiKey",
                "defaultModelSelection.modelId",
            ],
            bili: false,
            after_write: Some(
                "ZCode 会自动读取新配置。首次打开如果要求登录，可选“使用 API key”，再点“暂时跳过”。",
            ),
        }
    }

    fn read(&self, files: &[PathBuf]) -> Result<Current> {
        read(files)
    }

    fn apply(&self, files: &[PathBuf], settings: &Settings) -> Result<Prior> {
        apply(files, settings)
    }

    fn remove(&self, files: &[PathBuf], prior: &Prior) -> Result<()> {
        remove(files, prior)
    }

    fn sync_models(&self, files: &[PathBuf], model: &str, models: &[String]) -> Result<bool> {
        sync_models(files, model, models)
    }

    fn notice(&self, dir: &Path) -> Option<&'static str> {
        if moved(dir).is_some() {
            Some("zcode-moved")
        } else if !dir.join(FILE).exists() {
            Some("zcode-open-first")
        } else {
            None
        }
    }
}
