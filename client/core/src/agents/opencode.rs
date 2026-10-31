//! OpenCode reads `~/.config/opencode/opencode.json` on every system. OpenVAIR
//! adds an `openvair` provider listing the key's models and selects
//! `openvair/<model>`; other providers are left alone.
//!
//! A config kept as `opencode.jsonc` has comments OpenVAIR would lose, so OpenVAIR
//! leaves it to the user.
use super::{
    Current, Meta, Prior, Protocol, Settings, listed_models, lookup, object_mut, read_json,
    remember, text, write_json,
};
use crate::{CoreError, Result};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use url::Url;

const PROVIDER: &str = "openvair";
const MODEL: &str = "model";
const PREFIX: &str = "openvair/";

fn commented(path: &Path) -> bool {
    path.with_extension("jsonc").exists()
}

fn models(model: &str, models: &[String]) -> Value {
    Value::Object(
        listed_models(model, models)
            .into_iter()
            .map(|name| (name.clone(), json!({ "name": name })))
            .collect(),
    )
}

fn base_url(protocol: Protocol, server: &Url) -> String {
    // Both of OpenCode's SDKs expect `/v1` in the base URL.
    let root = server.as_str().trim_end_matches('/');
    match protocol {
        Protocol::OpenAi | Protocol::Anthropic => format!("{root}/v1"),
        Protocol::Gemini => root.to_owned(),
    }
}

pub fn read(files: &[PathBuf]) -> Result<Current> {
    let config = read_json(&files[0])?;
    let Some(provider) = lookup(&config, &["provider", PROVIDER]) else {
        return Ok(Current::default());
    };
    Ok(Current {
        base_url: text(provider.pointer("/options/baseURL")),
        secret: text(provider.pointer("/options/apiKey")),
        model: text(config.get(MODEL))
            .and_then(|model| model.strip_prefix(PREFIX).map(ToOwned::to_owned)),
    })
}

pub fn apply(files: &[PathBuf], settings: &Settings) -> Result<Prior> {
    let path = &files[0];
    if commented(path) {
        return Err(CoreError::Other(
            "OpenCode 的配置是 opencode.jsonc，OpenVAIR 改写会丢掉注释，请在“接入 Agent”里选“手动接入”"
                .into(),
        ));
    }
    let mut config = read_json(path)?;
    let mut prior = Prior::new();
    remember(&mut prior, MODEL, config.get(MODEL).cloned());
    if config.is_empty() {
        config.insert("$schema".into(), json!("https://opencode.ai/config.json"));
    }
    let protocol = settings.protocol.unwrap_or(Protocol::OpenAi);
    let npm = match protocol {
        Protocol::Anthropic => "@ai-sdk/anthropic",
        _ => "@ai-sdk/openai-compatible",
    };
    object_mut(&mut config, "provider").insert(
        PROVIDER.into(),
        json!({
            "npm": npm,
            "name": "OpenVAIR",
            "options": {
                "baseURL": base_url(protocol, settings.server),
                "apiKey": settings.secret,
            },
            "models": models(settings.model, settings.models),
        }),
    );
    config.insert(MODEL.into(), json!(format!("{PREFIX}{}", settings.model)));
    write_json(path, config)?;
    Ok(prior)
}

pub fn sync_models(files: &[PathBuf], model: &str, list: &[String]) -> Result<bool> {
    let path = &files[0];
    let mut config = read_json(path)?;
    let Some(provider) = config
        .get_mut("provider")
        .and_then(|providers| providers.get_mut(PROVIDER))
        .and_then(Value::as_object_mut)
    else {
        return Ok(false);
    };
    let next = models(model, list);
    if provider.get("models") == Some(&next) {
        return Ok(false);
    }
    provider.insert("models".into(), next);
    write_json(path, config)?;
    Ok(true)
}

pub fn remove(files: &[PathBuf], prior: &Prior) -> Result<()> {
    let path = &files[0];
    if !path.exists() {
        return Ok(());
    }
    let mut config: Map<String, Value> = read_json(path)?;
    if let Some(providers) = config.get_mut("provider").and_then(Value::as_object_mut) {
        providers.remove(PROVIDER);
        if providers.is_empty() {
            config.remove("provider");
        }
    }
    if text(config.get(MODEL)).is_some_and(|model| model.starts_with(PREFIX)) {
        match prior.get(MODEL).cloned().flatten() {
            Some(old) => {
                config.insert(MODEL.into(), old);
            }
            None => {
                config.remove(MODEL);
            }
        }
    }
    write_json(path, config)
}

pub struct OpenCode;

impl super::Adapter for OpenCode {
    fn meta(&self) -> &'static Meta {
        &Meta {
            name: "OpenCode",
            glyph: "OC",
            homepage: "https://opencode.ai/docs/",
            protocols: &[Protocol::OpenAi, Protocol::Anthropic],
            binaries: &["opencode"],
            traces: &[".local/share/opencode", "/Applications/OpenCode.app"],
            dir: ".config/opencode",
            files: &["opencode.json"],
            fields: [
                "provider.openvair.options.baseURL",
                "provider.openvair.options.apiKey",
                "model",
            ],
            bili: false,
            after_write: Some(
                "重新运行 opencode 后生效，也可以在 /models 里切到 OpenVAIR 的模型。项目里的 opencode.json 指定的模型会优先。",
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

    fn base_url(&self, protocol: Protocol, server: &Url) -> String {
        base_url(protocol, server)
    }

    fn sync_models(&self, files: &[PathBuf], model: &str, models: &[String]) -> Result<bool> {
        sync_models(files, model, models)
    }

    fn notice(&self, dir: &Path) -> Option<&'static str> {
        commented(&dir.join("opencode.json")).then_some("opencode-jsonc")
    }
}
