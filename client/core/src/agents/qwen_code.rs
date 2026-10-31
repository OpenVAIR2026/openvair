//! Qwen Code reads `~/.qwen/settings.json`. Its `modelProviders` lists
//! models per protocol, each naming the variable that holds its key, so OpenVAIR
//! adds rows that use `OPENVAIR_API_KEY`, sets that variable in the file's `env`
//! block and selects the protocol and model. Rows of other providers are
//! left alone.
//!
//! The key lives in `env` rather than `~/.qwen/.env` because Qwen Code
//! reads only the first `.env` it finds, and a project's own would win.
use super::{
    Current, Meta, Prior, Protocol, Settings, listed_models, lookup, object_mut, read_json,
    remember, text, write_json,
};
use crate::Result;
use serde_json::{Map, Value, json};
use std::path::PathBuf;

const KEY_VAR: &str = "OPENVAIR_API_KEY";
const AUTH_FIELD: &str = "security.auth.selectedType";
const MODEL_FIELD: &str = "model.name";
const GROUPS: [&str; 2] = ["openai", "anthropic"];

fn group(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::Anthropic => "anthropic",
        _ => "openai",
    }
}

fn is_ours(row: &Value) -> bool {
    row.get("envKey").and_then(Value::as_str) == Some(KEY_VAR)
}

/// OpenVAIR's rows, from whichever protocol holds them.
fn our_rows(config: &Map<String, Value>) -> Vec<&Value> {
    GROUPS
        .iter()
        .filter_map(|group| lookup(config, &["modelProviders", group])?.as_array())
        .flatten()
        .filter(|row| is_ours(row))
        .collect()
}

fn drop_rows(config: &mut Map<String, Value>) {
    let Some(providers) = config
        .get_mut("modelProviders")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    for group in GROUPS {
        if let Some(rows) = providers.get_mut(group).and_then(Value::as_array_mut) {
            rows.retain(|row| !is_ours(row));
            if rows.is_empty() {
                providers.remove(group);
            }
        }
    }
    if providers.is_empty() {
        config.remove("modelProviders");
    }
}

fn set_rows(
    config: &mut Map<String, Value>,
    protocol: Protocol,
    base_url: &str,
    model: &str,
    models: &[String],
) {
    drop_rows(config);
    let providers = object_mut(config, "modelProviders");
    let rows = providers
        .entry(group(protocol))
        .or_insert_with(|| Value::Array(Vec::new()));
    if !rows.is_array() {
        *rows = Value::Array(Vec::new());
    }
    let rows = rows.as_array_mut().expect("an array");
    rows.extend(listed_models(model, models).into_iter().map(|name| {
        json!({
            "id": name,
            "name": format!("{name} · OpenVAIR"),
            "baseUrl": base_url,
            "envKey": KEY_VAR,
        })
    }));
}

fn set_path(config: &mut Map<String, Value>, field: &str, new: Option<Value>) {
    let parts: Vec<&str> = field.split('.').collect();
    let (last, parents) = parts.split_last().expect("a field");
    match new {
        Some(value) => {
            parents
                .iter()
                .fold(config, |map, key| object_mut(map, key))
                .insert((*last).into(), value);
        }
        None => unset(config, &parts),
    }
}

/// Removes `path`, and the objects it leaves empty, without adding any.
fn unset(map: &mut Map<String, Value>, path: &[&str]) {
    let Some((first, rest)) = path.split_first() else {
        return;
    };
    if rest.is_empty() {
        map.remove(*first);
        return;
    }
    let Some(next) = map.get_mut(*first).and_then(Value::as_object_mut) else {
        return;
    };
    unset(next, rest);
    if next.is_empty() {
        map.remove(*first);
    }
}

fn get_path<'a>(config: &'a Map<String, Value>, field: &str) -> Option<&'a Value> {
    lookup(config, &field.split('.').collect::<Vec<_>>())
}

pub fn read(files: &[PathBuf]) -> Result<Current> {
    let config = read_json(&files[0])?;
    let rows = our_rows(&config);
    let Some(first) = rows.first() else {
        return Ok(Current::default());
    };
    let model = text(get_path(&config, MODEL_FIELD)).filter(|model| {
        rows.iter()
            .any(|row| row.get("id").and_then(Value::as_str) == Some(model))
    });
    Ok(Current {
        base_url: text(first.get("baseUrl")),
        secret: text(lookup(&config, &["env", KEY_VAR])),
        model,
    })
}

pub fn apply(files: &[PathBuf], settings: &Settings) -> Result<Prior> {
    let path = &files[0];
    let mut config = read_json(path)?;
    let mut prior = Prior::new();
    for field in [AUTH_FIELD, MODEL_FIELD] {
        remember(&mut prior, field, get_path(&config, field).cloned());
    }
    let protocol = settings.protocol.unwrap_or(Protocol::OpenAi);
    let base_url = super::protocol_url(protocol, settings.server);
    set_rows(
        &mut config,
        protocol,
        &base_url,
        settings.model,
        settings.models,
    );
    object_mut(&mut config, "env").insert(KEY_VAR.into(), json!(settings.secret));
    set_path(&mut config, AUTH_FIELD, Some(json!(group(protocol))));
    set_path(&mut config, MODEL_FIELD, Some(json!(settings.model)));
    write_json(path, config)?;
    Ok(prior)
}

pub fn sync_models(files: &[PathBuf], model: &str, models: &[String]) -> Result<bool> {
    let path = &files[0];
    let mut config = read_json(path)?;
    let Some((protocol, base_url)) = GROUPS.iter().find_map(|name| {
        let row = lookup(&config, &["modelProviders", name])?
            .as_array()?
            .iter()
            .find(|row| is_ours(row))?;
        let protocol = if *name == "anthropic" {
            Protocol::Anthropic
        } else {
            Protocol::OpenAi
        };
        Some((protocol, text(row.get("baseUrl"))?))
    }) else {
        return Ok(false);
    };
    let before = config.clone();
    set_rows(&mut config, protocol, &base_url, model, models);
    if config == before {
        return Ok(false);
    }
    write_json(path, config)?;
    Ok(true)
}

pub fn remove(files: &[PathBuf], prior: &Prior) -> Result<()> {
    let path = &files[0];
    if !path.exists() {
        return Ok(());
    }
    let mut config = read_json(path)?;
    drop_rows(&mut config);
    if let Some(env) = config.get_mut("env").and_then(Value::as_object_mut) {
        env.remove(KEY_VAR);
        if env.is_empty() {
            config.remove("env");
        }
    }
    for field in [AUTH_FIELD, MODEL_FIELD] {
        set_path(&mut config, field, prior.get(field).cloned().flatten());
    }
    write_json(path, config)
}

pub struct QwenCode;

impl super::Adapter for QwenCode {
    fn meta(&self) -> &'static Meta {
        &Meta {
            name: "Qwen Code",
            glyph: "Q",
            homepage: "https://github.com/QwenLM/qwen-code",
            protocols: &[Protocol::OpenAi, Protocol::Anthropic],
            binaries: &["qwen"],
            traces: &[".qwen/tmp", ".qwen/installation_id"],
            dir: ".qwen",
            files: &["settings.json"],
            fields: [
                "modelProviders[].baseUrl",
                "env.OPENVAIR_API_KEY",
                MODEL_FIELD,
            ],
            bili: false,
            after_write: Some(
                "重新运行 qwen 后生效。项目里的 .qwen/settings.json 如果指定了别的模型，会优先于这里的配置。",
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
}
