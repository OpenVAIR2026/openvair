//! Claude Code reads environment overrides from the `env` block of
//! `~/.claude/settings.json`. `ANTHROPIC_AUTH_TOKEN` is sent as a bearer
//! token, which the OpenVAIR gateway accepts.
//!
//! The `/model` picker only lists Claude models, so the key's other models
//! are added as `modelPicker` rows. A lineup the user wrote is left alone.
//!
//! Claude Code sends background work (titles, compaction, WebFetch
//! summaries) to the `haiku` alias, and `sonnet`/`opus` to their aliases.
//! A key without Claude models cannot serve those, so the aliases are
//! pointed at the key's models, only on connect, where the preview shows it.
use super::{Change, Current, Settings, base_url, read_json, remember, write_json};
use crate::Result;
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

const FIELDS: [&str; 3] = [
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_MODEL",
];

/// Background work uses the first; the others follow the chosen model.
const ALIASES: [&str; 3] = [
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
];

const PICKER: &str = "modelPicker";
const ROW_NOTE: &str = "通过 OpenVAIR 使用";

/// Built-in rows and gateway discovery already cover Claude models.
fn is_claude(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    model.contains("claude") || model.contains("anthropic")
}

/// Only rows OpenVAIR wrote, adding to the built-in lineup.
fn is_ours(picker: &Value) -> bool {
    let Some(picker) = picker.as_object() else {
        return false;
    };
    picker.keys().all(|key| key == "options")
        && picker
            .get("options")
            .and_then(Value::as_array)
            .is_some_and(|rows| {
                rows.iter()
                    .all(|row| row.get("description").and_then(Value::as_str) == Some(ROW_NOTE))
            })
}

fn set_picker(config: &mut Map<String, Value>, model: &str, models: &[String]) {
    if config.get(PICKER).is_some_and(|picker| !is_ours(picker)) {
        return;
    }
    let models: BTreeSet<&str> = models
        .iter()
        .map(String::as_str)
        .chain([model])
        .filter(|model| !model.is_empty() && !is_claude(model))
        .collect();
    if models.is_empty() {
        config.remove(PICKER);
        return;
    }
    let rows = models
        .into_iter()
        .map(|model| json!({ "model": model, "description": ROW_NOTE }))
        .collect();
    config.insert(PICKER.into(), json!({ "options": Value::Array(rows) }));
}

/// The aliases the key cannot serve: all of them when it has no Claude
/// models, otherwise those the user pointed at a model it does not list.
/// An empty list tells nothing, so nothing is changed.
fn stale_aliases(config: &Map<String, Value>, models: &[String]) -> Vec<&'static str> {
    if models.is_empty() {
        return Vec::new();
    }
    if !models.iter().any(|model| is_claude(model)) {
        return ALIASES.to_vec();
    }
    ALIASES
        .into_iter()
        .filter(|field| {
            env_string(config, field)
                .is_some_and(|value| !is_claude(&value) && !models.contains(&value))
        })
        .collect()
}

/// The alias values connecting writes; background work defaults to the
/// chosen model.
fn alias_values(
    config: &Map<String, Value>,
    model: &str,
    background: Option<&str>,
    models: &[String],
) -> Vec<(&'static str, String)> {
    stale_aliases(config, models)
        .into_iter()
        .map(|field| {
            let value = match background {
                Some(background) if field == ALIASES[0] && !background.is_empty() => background,
                _ => model,
            };
            (field, value.to_owned())
        })
        .collect()
}

/// Whether connecting with these models changes the background model, so
/// the user should pick it.
pub fn asks_background_model(files: &[PathBuf], models: &[String]) -> Result<bool> {
    let config = read_json(&files[0])?;
    Ok(stale_aliases(&config, models).contains(&ALIASES[0]))
}

fn picker_models(picker: Option<&Value>) -> Option<String> {
    let rows = picker?.get("options")?.as_array()?;
    let names: Vec<&str> = rows
        .iter()
        .filter_map(|row| row.get("model").and_then(Value::as_str))
        .collect();
    Some(names.join(", "))
}

/// Preview lines for the aliases and the picker.
pub fn preview(files: &[PathBuf], settings: &Settings) -> Result<Vec<Change>> {
    let mut config = read_json(&files[0])?;
    let mut changes: Vec<Change> = alias_values(
        &config,
        settings.model,
        settings.background_model,
        settings.models,
    )
    .into_iter()
    .map(|(field, after)| Change {
        field: format!("env.{field}"),
        before: env_string(&config, field),
        after: Some(after),
    })
    .collect();
    let before = picker_models(config.get(PICKER));
    set_picker(&mut config, settings.model, settings.models);
    let after = picker_models(config.get(PICKER));
    if before.is_some() || after.is_some() {
        changes.push(Change {
            field: format!("{PICKER}（/model 菜单）"),
            before,
            after,
        });
    }
    Ok(changes)
}

fn env_string(settings: &Map<String, Value>, field: &str) -> Option<String> {
    settings
        .get("env")
        .and_then(|env| env.get(field))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

pub fn read(files: &[PathBuf]) -> Result<Current> {
    let settings = read_json(&files[0])?;
    Ok(Current {
        base_url: env_string(&settings, FIELDS[0]),
        secret: env_string(&settings, FIELDS[1]),
        model: env_string(&settings, FIELDS[2]),
    })
}

pub fn apply(files: &[PathBuf], settings: &Settings) -> Result<BTreeMap<String, Option<Value>>> {
    let path = &files[0];
    let mut config = read_json(path)?;
    let mut prior = BTreeMap::new();
    let env = config
        .entry("env")
        .or_insert_with(|| Value::Object(Map::new()));
    if !env.is_object() {
        *env = Value::Object(Map::new());
    }
    let env = env.as_object_mut().expect("env was just made an object");
    // A bili-managed address keeps routing through bili to the new server.
    let current = env.get(FIELDS[0]).and_then(Value::as_str);
    let values = [
        crate::bili::keep_wrapping(
            current,
            &base_url(super::AgentId::ClaudeCode, settings.server),
        ),
        settings.secret.to_owned(),
        settings.model.to_owned(),
    ];
    for (field, new) in FIELDS.into_iter().zip(values) {
        remember(&mut prior, field, env.get(field).cloned());
        env.insert(field.into(), Value::String(new));
    }
    let aliases = alias_values(
        &config,
        settings.model,
        settings.background_model,
        settings.models,
    );
    let env = config
        .get_mut("env")
        .and_then(Value::as_object_mut)
        .expect("env was just made an object");
    for (field, new) in aliases {
        remember(&mut prior, field, env.get(field).cloned());
        env.insert(field.into(), Value::String(new));
    }
    set_picker(&mut config, settings.model, settings.models);
    write_json(path, config)?;
    Ok(prior)
}

/// Brings the picker in line with the key's current models. Returns whether
/// the file changed.
pub fn sync_models(files: &[PathBuf], model: &str, models: &[String]) -> Result<bool> {
    let path = &files[0];
    if !path.exists() {
        return Ok(false);
    }
    let mut config = read_json(path)?;
    if env_string(&config, FIELDS[1]).is_none() {
        return Ok(false);
    }
    let before = config.get(PICKER).cloned();
    set_picker(&mut config, model, models);
    if config.get(PICKER) == before.as_ref() {
        return Ok(false);
    }
    write_json(path, config)?;
    Ok(true)
}

pub fn remove(files: &[PathBuf], prior: &BTreeMap<String, Option<Value>>) -> Result<()> {
    let path = &files[0];
    if !path.exists() {
        return Ok(());
    }
    let mut config = read_json(path)?;
    if let Some(env) = config.get_mut("env").and_then(Value::as_object_mut) {
        // Aliases are only in `prior` when connecting changed them.
        let aliases = ALIASES
            .into_iter()
            .filter(|field| prior.contains_key(*field));
        for field in FIELDS.into_iter().chain(aliases) {
            match prior.get(field).cloned().flatten() {
                Some(old) => {
                    env.insert(field.into(), old);
                }
                None => {
                    env.remove(field);
                }
            }
        }
        if env.is_empty() {
            config.remove("env");
        }
    }
    if config.get(PICKER).is_some_and(is_ours) {
        config.remove(PICKER);
    }
    write_json(path, config)
}

pub struct ClaudeCode;

impl super::Adapter for ClaudeCode {
    fn meta(&self) -> &'static super::Meta {
        &super::Meta {
            name: "Claude Code",
            glyph: "CC",
            homepage: "https://docs.anthropic.com/claude-code",
            protocols: &[super::Protocol::Anthropic],
            binaries: &["claude"],
            traces: &[".claude.json", ".claude/projects"],
            dir: ".claude",
            files: &["settings.json"],
            fields: [
                "env.ANTHROPIC_BASE_URL",
                "env.ANTHROPIC_AUTH_TOKEN",
                "env.ANTHROPIC_MODEL",
            ],
            bili: true,
            after_write: Some("新开一个 Claude Code 会话后生效，正在运行的会话仍用原来的配置。"),
        }
    }

    fn read(&self, files: &[PathBuf]) -> Result<Current> {
        read(files)
    }

    fn apply(&self, files: &[PathBuf], settings: &Settings) -> Result<super::Prior> {
        apply(files, settings)
    }

    fn remove(&self, files: &[PathBuf], prior: &super::Prior) -> Result<()> {
        remove(files, prior)
    }

    fn preview(&self, files: &[PathBuf], settings: &Settings) -> Result<Vec<Change>> {
        preview(files, settings)
    }

    fn sync_models(&self, files: &[PathBuf], model: &str, models: &[String]) -> Result<bool> {
        sync_models(files, model, models)
    }

    fn asks_background_model(&self, files: &[PathBuf], models: &[String]) -> Result<bool> {
        asks_background_model(files, models)
    }
}
