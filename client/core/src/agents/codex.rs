//! Codex reads `~/.codex/config.toml`. OpenVAIR adds an `openvair` model provider and
//! selects it; the rest of the file, comments included, is left as it was.
//! When the key has models Codex does not know, it also points
//! `model_catalog_json` at a catalog listing them (see `codex_models`).
use super::{
    Change, Current, Settings, base_url, codex_models, read_json, read_text, remember, write_text,
};
use crate::{CoreError, Result};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use toml_edit::{DocumentMut, Item, Table, value};

const PROVIDER: &str = "openvair";
const CATALOG: &str = "model_catalog_json";

fn load(path: &Path) -> Result<DocumentMut> {
    read_text(path)?
        .unwrap_or_default()
        .parse::<DocumentMut>()
        .map_err(|_| CoreError::InvalidConfig(path.display().to_string()))
}

fn string(item: Option<&Item>) -> Option<String> {
    item.and_then(Item::as_str).map(ToOwned::to_owned)
}

pub fn read(files: &[PathBuf]) -> Result<Current> {
    let doc = load(&files[0])?;
    if doc.get("model_provider").and_then(Item::as_str) != Some(PROVIDER) {
        return Ok(Current::default());
    }
    let provider = doc
        .get("model_providers")
        .and_then(|providers| providers.get(PROVIDER));
    Ok(Current {
        base_url: string(provider.and_then(|p| p.get("base_url"))),
        secret: string(provider.and_then(|p| p.get("experimental_bearer_token"))),
        model: string(doc.get("model")),
    })
}

pub fn apply(files: &[PathBuf], settings: &Settings) -> Result<BTreeMap<String, Option<Value>>> {
    let path = &files[0];
    let mut doc = load(path)?;
    let mut prior = BTreeMap::new();
    for field in ["model", "model_provider"] {
        remember(&mut prior, field, string(doc.get(field)).map(Value::String));
    }
    doc["model_provider"] = value(PROVIDER);
    doc["model"] = value(settings.model);
    if !doc.contains_table("model_providers") {
        let mut providers = Table::new();
        providers.set_implicit(true);
        doc["model_providers"] = Item::Table(providers);
    }
    // A bili-managed address keeps routing through bili to the new server.
    let current = string(
        doc.get("model_providers")
            .and_then(|providers| providers.get(PROVIDER))
            .and_then(|provider| provider.get("base_url")),
    );
    let mut provider = Table::new();
    provider["name"] = value("OpenVAIR");
    provider["base_url"] = value(crate::bili::keep_wrapping(
        current.as_deref(),
        &base_url(super::AgentId::Codex, settings.server),
    ));
    provider["wire_api"] = value("responses");
    provider["experimental_bearer_token"] = value(settings.secret);
    doc["model_providers"][PROVIDER] = Item::Table(provider);
    set_catalog(
        &mut doc,
        &files[1],
        settings.model,
        settings.models,
        &mut prior,
    )?;
    write_text(path, &doc.to_string())?;
    Ok(prior)
}

fn is_ours(doc: &DocumentMut, ours: &Path) -> bool {
    string(doc.get(CATALOG)).is_some_and(|current| Path::new(&current) == ours)
}

/// A catalog the user set up themselves is left alone.
fn set_catalog(
    doc: &mut DocumentMut,
    ours: &Path,
    model: &str,
    models: &[String],
    prior: &mut BTreeMap<String, Option<Value>>,
) -> Result<bool> {
    if doc.contains_key(CATALOG) && !is_ours(doc, ours) {
        return Ok(false);
    }
    remember(prior, CATALOG, None);
    let dir = ours.parent().unwrap_or(Path::new("."));
    let wanted: BTreeSet<String> = models.iter().cloned().chain([model.to_owned()]).collect();
    let written = codex_models::write(dir, &wanted)?;
    match written.path {
        // Top-level keys go before the first table, where `doc[...]` puts them.
        Some(catalog) => doc[CATALOG] = value(catalog.display().to_string()),
        None => {
            doc.remove(CATALOG);
        }
    }
    Ok(written.changed)
}

/// The preview line for the catalog, unless the user set up their own.
pub fn preview(files: &[PathBuf], settings: &Settings) -> Result<Vec<Change>> {
    let doc = load(&files[0])?;
    if doc.contains_key(CATALOG) && !is_ours(&doc, &files[1]) {
        return Ok(Vec::new());
    }
    let dir = files[1].parent().unwrap_or(Path::new("."));
    let wanted: BTreeSet<String> = settings
        .models
        .iter()
        .cloned()
        .chain([settings.model.to_owned()])
        .collect();
    let added = codex_models::listed(dir, &wanted)?;
    let before = string(doc.get(CATALOG));
    let after =
        (!added.is_empty()).then(|| format!("{}（加入 {}）", files[1].display(), added.join(", ")));
    if before.is_none() && after.is_none() {
        return Ok(Vec::new());
    }
    Ok(vec![Change {
        field: CATALOG.into(),
        before,
        after,
    }])
}

/// Brings the catalog in line with the key's current models. Returns whether
/// anything changed.
pub fn sync_models(files: &[PathBuf], model: &str, models: &[String]) -> Result<bool> {
    let path = &files[0];
    let Some(text) = read_text(path)? else {
        return Ok(false);
    };
    let mut doc = load(path)?;
    if doc.get("model_provider").and_then(Item::as_str) != Some(PROVIDER) {
        return Ok(false);
    }
    let catalog = set_catalog(&mut doc, &files[1], model, models, &mut BTreeMap::new())?;
    let config = doc.to_string() != text;
    if config {
        write_text(path, &doc.to_string())?;
    }
    Ok(catalog || config)
}

pub fn remove(files: &[PathBuf], prior: &BTreeMap<String, Option<Value>>) -> Result<()> {
    let path = &files[0];
    if !path.exists() {
        return Ok(());
    }
    let mut doc = load(path)?;
    if let Some(providers) = doc
        .get_mut("model_providers")
        .and_then(Item::as_table_like_mut)
    {
        providers.remove(PROVIDER);
        if providers.is_empty() {
            doc.remove("model_providers");
        }
    }
    for field in ["model", "model_provider"] {
        match prior.get(field).cloned().flatten() {
            Some(Value::String(old)) => doc[field] = value(old),
            _ => {
                doc.remove(field);
            }
        }
    }
    if is_ours(&doc, &files[1]) {
        doc.remove(CATALOG);
    }
    codex_models::remove(files[1].parent().unwrap_or(Path::new(".")));
    write_text(path, &doc.to_string())
}

/// Signed in with ChatGPT, Codex asks chatgpt.com which backend serves the
/// workspace before each turn, even when requests go to OpenVAIR; if that times
/// out the turn fails with "workspace routing discovery timed out". Only
/// `auth_mode` is read, never the tokens.
pub fn notice(dir: &Path) -> Option<&'static str> {
    let auth = read_json(&dir.join("auth.json")).ok()?;
    (auth.get("auth_mode").and_then(Value::as_str) == Some("chatgpt"))
        .then_some("codex-chatgpt-login")
}

pub struct Codex;

impl super::Adapter for Codex {
    fn meta(&self) -> &'static super::Meta {
        &super::Meta {
            name: "Codex",
            glyph: ">_",
            homepage: "https://github.com/openai/codex",
            protocols: &[super::Protocol::OpenAi],
            binaries: &["codex"],
            traces: &[
                ".codex/auth.json",
                ".codex/sessions",
                "/Applications/Codex.app",
            ],
            dir: ".codex",
            files: &["config.toml", codex_models::FILE],
            fields: [
                "model_providers.openvair.base_url",
                "model_providers.openvair.experimental_bearer_token",
                "model",
            ],
            bili: true,
            after_write: Some("新开一个 Codex 会话后生效；在 IDE 插件里用的话，重新加载窗口。"),
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

    fn notice(&self, dir: &Path) -> Option<&'static str> {
        notice(dir)
    }

    fn refresh(&self, dir: &Path) -> Result<bool> {
        codex_models::refresh(dir)
    }
}
