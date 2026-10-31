//! Kimi CLI reads `~/.kimi/config.toml`. OpenVAIR adds an `openvair` provider and a
//! `openvair/<model>` entry for each of the key's models, and points
//! `default_model` at the chosen one; the rest of the file, comments
//! included, is left as it was.
//!
//! Kimi CLI refuses to start when `default_model` names no entry, so
//! disconnecting puts the old value back whenever OpenVAIR's is still there.
use super::{
    Current, Meta, Prior, Protocol, Settings, listed_models, read_text, remember, write_text,
};
use crate::{CoreError, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Table, TableLike, value};

const PROVIDER: &str = "openvair";
const DEFAULT: &str = "default_model";
/// Kimi CLI compacts by this size; the key does not say, so most models'.
const CONTEXT: i64 = 131_072;

fn entry(model: &str) -> String {
    format!("{PROVIDER}/{model}")
}

fn kind(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::OpenAi => "openai_legacy",
        Protocol::Anthropic => "anthropic",
        Protocol::Gemini => "gemini",
    }
}

fn load(path: &Path) -> Result<DocumentMut> {
    read_text(path)?
        .unwrap_or_default()
        .parse::<DocumentMut>()
        .map_err(|_| CoreError::InvalidConfig(path.display().to_string()))
}

fn string(table: &dyn TableLike, key: &str) -> Option<String> {
    table.get(key).and_then(Item::as_str).map(ToOwned::to_owned)
}

fn provider(doc: &DocumentMut) -> Option<&dyn TableLike> {
    doc.get("providers")?.get(PROVIDER)?.as_table_like()
}

fn is_ours(table: &dyn TableLike) -> bool {
    table.get("provider").and_then(Item::as_str) == Some(PROVIDER)
}

/// A table of tables, created without a header of its own.
fn tables<'a>(doc: &'a mut DocumentMut, key: &str, path: &Path) -> Result<&'a mut Table> {
    doc.entry(key)
        .or_insert_with(|| {
            let mut table = Table::new();
            table.set_implicit(true);
            Item::Table(table)
        })
        .as_table_mut()
        .ok_or_else(|| CoreError::InvalidConfig(path.display().to_string()))
}

fn set_models(doc: &mut DocumentMut, path: &Path, model: &str, models: &[String]) -> Result<()> {
    let table = tables(doc, "models", path)?;
    table.retain(|_, item| !item.as_table_like().is_some_and(is_ours));
    for name in listed_models(model, models) {
        let mut row = Table::new();
        row["provider"] = value(PROVIDER);
        row["model"] = value(name.as_str());
        row["max_context_size"] = value(CONTEXT);
        table.insert(&entry(&name), Item::Table(row));
    }
    Ok(())
}

pub fn read(files: &[PathBuf]) -> Result<Current> {
    let doc = load(&files[0])?;
    let Some(provider) = provider(&doc) else {
        return Ok(Current::default());
    };
    let model = doc
        .get(DEFAULT)
        .and_then(Item::as_str)
        .and_then(|name| doc.get("models")?.get(name)?.as_table_like())
        .filter(|row| is_ours(*row))
        .and_then(|row| string(row, "model"));
    Ok(Current {
        base_url: string(provider, "base_url"),
        secret: string(provider, "api_key"),
        model,
    })
}

pub fn apply(files: &[PathBuf], settings: &Settings) -> Result<Prior> {
    let path = &files[0];
    let mut doc = load(path)?;
    let mut prior = Prior::new();
    remember(
        &mut prior,
        DEFAULT,
        doc.get(DEFAULT)
            .and_then(Item::as_str)
            .map(|old| Value::String(old.to_owned())),
    );
    let protocol = settings.protocol.unwrap_or(Protocol::OpenAi);
    let mut table = Table::new();
    table["type"] = value(kind(protocol));
    table["base_url"] = value(super::protocol_url(protocol, settings.server));
    table["api_key"] = value(settings.secret);
    tables(&mut doc, "providers", path)?.insert(PROVIDER, Item::Table(table));
    set_models(&mut doc, path, settings.model, settings.models)?;
    doc[DEFAULT] = value(entry(settings.model));
    write_text(path, &doc.to_string())?;
    Ok(prior)
}

pub fn sync_models(files: &[PathBuf], model: &str, models: &[String]) -> Result<bool> {
    let path = &files[0];
    let mut doc = load(path)?;
    if provider(&doc).is_none() {
        return Ok(false);
    }
    let before = doc.to_string();
    set_models(&mut doc, path, model, models)?;
    let after = doc.to_string();
    if after == before {
        return Ok(false);
    }
    write_text(path, &after)?;
    Ok(true)
}

pub fn remove(files: &[PathBuf], prior: &Prior) -> Result<()> {
    let path = &files[0];
    if !path.exists() {
        return Ok(());
    }
    let mut doc = load(path)?;
    if let Some(providers) = doc.get_mut("providers").and_then(Item::as_table_like_mut) {
        providers.remove(PROVIDER);
        if providers.is_empty() {
            doc.remove("providers");
        }
    }
    if let Some(models) = doc.get_mut("models").and_then(Item::as_table_mut) {
        models.retain(|_, item| !item.as_table_like().is_some_and(is_ours));
        if models.is_empty() {
            doc.remove("models");
        }
    }
    let ours = doc
        .get(DEFAULT)
        .and_then(Item::as_str)
        .is_some_and(|name| name.starts_with(&entry("")));
    if ours {
        match prior.get(DEFAULT).cloned().flatten() {
            Some(Value::String(old)) => doc[DEFAULT] = value(old),
            _ => {
                doc.remove(DEFAULT);
            }
        }
    }
    write_text(path, &doc.to_string())
}

pub struct KimiCli;

impl super::Adapter for KimiCli {
    fn meta(&self) -> &'static Meta {
        &Meta {
            name: "Kimi CLI",
            glyph: "K",
            homepage: "https://github.com/MoonshotAI/kimi-cli",
            protocols: &[Protocol::OpenAi, Protocol::Anthropic],
            binaries: &["kimi"],
            traces: &[
                ".kimi/sessions",
                ".kimi/kimi.json",
                ".local/share/uv/tools/kimi-cli",
            ],
            dir: ".kimi",
            files: &["config.toml"],
            fields: [
                "providers.openvair.base_url",
                "providers.openvair.api_key",
                "default_model",
            ],
            bili: false,
            after_write: Some(
                "重新运行 kimi 后生效。用 OpenAI 协议时，终端里的 OPENAI_BASE_URL、OPENAI_API_KEY 会覆盖这里的地址和 Key。",
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
