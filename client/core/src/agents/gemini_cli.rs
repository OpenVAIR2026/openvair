//! Gemini CLI loads `~/.gemini/.env` and needs API-key sign-in selected in
//! `~/.gemini/settings.json`, otherwise it asks to sign in with Google.
use super::{Current, Settings, base_url, read_json, read_text, remember, write_json, write_text};
use crate::Result;
use serde_json::{Map, Value};
use std::{collections::BTreeMap, path::PathBuf};

const FIELDS: [&str; 3] = ["GOOGLE_GEMINI_BASE_URL", "GEMINI_API_KEY", "GEMINI_MODEL"];
const AUTH_FIELD: &str = "settings:security.auth.selectedType";

fn parse_line(line: &str) -> Option<(&str, String)> {
    let line = line.trim();
    let line = line.strip_prefix("export ").unwrap_or(line);
    if line.starts_with('#') {
        return None;
    }
    let (key, value) = line.split_once('=')?;
    let value = value.trim();
    let value = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
        .unwrap_or(value);
    Some((key.trim(), value.to_owned()))
}

pub(super) fn env_values(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(parse_line)
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

/// Replaces or appends `KEY="value"` lines, keeping every other line as is.
pub(super) fn set_env(text: &str, updates: &BTreeMap<&str, Option<String>>) -> String {
    let mut seen = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        match parse_line(line).map(|(key, _)| key.to_owned()) {
            Some(key) if updates.contains_key(key.as_str()) => {
                if let Some(Some(new)) = updates.get(key.as_str()) {
                    lines.push(format!("{key}=\"{new}\""));
                }
                seen.push(key);
            }
            _ => lines.push(line.to_owned()),
        }
    }
    for (key, new) in updates {
        if let Some(new) = new.as_ref().filter(|_| !seen.iter().any(|s| s == key)) {
            lines.push(format!("{key}=\"{new}\""));
        }
    }
    let mut text = lines.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    text
}

pub fn read(files: &[PathBuf]) -> Result<Current> {
    let values = env_values(&read_text(&files[0])?.unwrap_or_default());
    Ok(Current {
        base_url: values.get(FIELDS[0]).cloned(),
        secret: values.get(FIELDS[1]).cloned(),
        model: values.get(FIELDS[2]).cloned(),
    })
}

fn auth_type(settings: &Map<String, Value>) -> Option<Value> {
    settings
        .get("security")?
        .get("auth")?
        .get("selectedType")
        .cloned()
}

fn set_auth_type(settings: &mut Map<String, Value>, new: Option<Value>) {
    let security = settings
        .entry("security")
        .or_insert_with(|| Value::Object(Map::new()));
    if !security.is_object() {
        *security = Value::Object(Map::new());
    }
    let auth = security
        .as_object_mut()
        .expect("security is an object")
        .entry("auth")
        .or_insert_with(|| Value::Object(Map::new()));
    if !auth.is_object() {
        *auth = Value::Object(Map::new());
    }
    let auth = auth.as_object_mut().expect("auth is an object");
    match new {
        Some(value) => {
            auth.insert("selectedType".into(), value);
        }
        None => {
            auth.remove("selectedType");
        }
    }
}

pub fn apply(files: &[PathBuf], settings: &Settings) -> Result<BTreeMap<String, Option<Value>>> {
    let text = read_text(&files[0])?.unwrap_or_default();
    let existing = env_values(&text);
    let mut prior = BTreeMap::new();
    let values = [
        base_url(super::AgentId::GeminiCli, settings.server),
        settings.secret.to_owned(),
        settings.model.to_owned(),
    ];
    let mut updates = BTreeMap::new();
    for (field, new) in FIELDS.into_iter().zip(values) {
        remember(
            &mut prior,
            field,
            existing.get(field).cloned().map(Value::String),
        );
        updates.insert(field, Some(new));
    }
    let mut config = read_json(&files[1])?;
    remember(&mut prior, AUTH_FIELD, auth_type(&config));
    set_auth_type(&mut config, Some(Value::String("gemini-api-key".into())));
    write_text(&files[0], &set_env(&text, &updates))?;
    write_json(&files[1], config)?;
    Ok(prior)
}

pub fn remove(files: &[PathBuf], prior: &BTreeMap<String, Option<Value>>) -> Result<()> {
    if files[0].exists() {
        let text = read_text(&files[0])?.unwrap_or_default();
        let updates: BTreeMap<&str, Option<String>> = FIELDS
            .into_iter()
            .map(|field| {
                let old = prior
                    .get(field)
                    .cloned()
                    .flatten()
                    .and_then(|v| v.as_str().map(ToOwned::to_owned));
                (field, old)
            })
            .collect();
        write_text(&files[0], &set_env(&text, &updates))?;
    }
    if files[1].exists() {
        let mut config = read_json(&files[1])?;
        set_auth_type(&mut config, prior.get(AUTH_FIELD).cloned().flatten());
        write_json(&files[1], config)?;
    }
    Ok(())
}

pub struct GeminiCli;

impl super::Adapter for GeminiCli {
    fn meta(&self) -> &'static super::Meta {
        &super::Meta {
            name: "Gemini CLI",
            glyph: "G",
            homepage: "https://github.com/google-gemini/gemini-cli",
            protocols: &[super::Protocol::Gemini],
            binaries: &["gemini"],
            traces: &[".gemini/tmp", ".gemini/installation_id"],
            dir: ".gemini",
            files: &[".env", "settings.json"],
            fields: FIELDS,
            bili: false,
            after_write: Some(
                "重新运行 gemini 后生效。终端里已设置的 GEMINI_API_KEY、GOOGLE_GEMINI_BASE_URL，或项目里的 .gemini/.env，会优先于这里的配置。",
            ),
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
}
