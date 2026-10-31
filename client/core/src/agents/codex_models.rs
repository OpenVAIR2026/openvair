//! Codex lists only the models in its catalog, which it downloads from
//! ChatGPT and caches in `~/.codex/models_cache.json`, or else takes from the
//! one it ships. A `model_catalog_json` file replaces that catalog, and every
//! entry needs Codex's own prompt, so OpenVAIR writes one built from both: Codex's
//! models as they are, plus the key's other vendors' models, copied from one
//! of them. OpenAI models Codex leaves out are older or not for coding, so
//! they stay out.
use super::{codex_bundled, read_json, write_json};
use crate::Result;
use serde_json::{Map, Value, json};
use std::{collections::BTreeSet, path::Path};

pub const FILE: &str = "openvair-models.json";
const CACHE: &str = "models_cache.json";
/// OpenVAIR's own record in the catalog; Codex ignores fields it does not know.
const META: &str = "openvair";

/// What OpenVAIR knows about a family of models Codex was not written for.
struct Family {
    prefix: &'static str,
    context: u64,
    efforts: &'static [&'static str],
    default_effort: &'static str,
}

const FAMILIES: &[Family] = &[
    Family {
        prefix: "deepseek-",
        context: 1_048_576,
        efforts: &["low", "high", "max"],
        default_effort: "high",
    },
    Family {
        prefix: "kimi-",
        context: 262_144,
        efforts: &["low", "medium", "high"],
        default_effort: "medium",
    },
    Family {
        prefix: "glm-",
        context: 200_000,
        efforts: &["low", "medium", "high"],
        default_effort: "medium",
    },
    Family {
        prefix: "minimax-",
        context: 204_800,
        efforts: &["low", "medium", "high"],
        default_effort: "medium",
    },
];

const UNKNOWN: Family = Family {
    prefix: "",
    context: 128_000,
    efforts: &["low", "medium", "high"],
    default_effort: "medium",
};

/// Promotions, paid speed tiers and access gates belong to OpenAI's models.
const OPENAI_ONLY: [&str; 5] = [
    "upgrade",
    "availability_nux",
    "service_tiers",
    "additional_speed_tiers",
    "available_access_programs",
];

fn is_openai(slug: &str) -> bool {
    let slug = slug.to_ascii_lowercase();
    slug.starts_with("gpt-")
        || slug.contains("codex")
        || (slug.starts_with('o') && slug[1..].starts_with(|c: char| c.is_ascii_digit()))
}

fn effort(name: &str) -> Value {
    let description = match name {
        "low" => "Fast responses with lighter reasoning",
        "medium" => "Balances speed and reasoning depth",
        "high" => "Greater reasoning depth for complex problems",
        _ => "Maximum reasoning depth for the hardest problems",
    };
    json!({ "effort": name, "description": description })
}

/// Copies `template` for `slug` with the tool settings DeepSeek documents
/// for Codex: plain shell, no code mode and text input only, since not every
/// model takes images.
fn copy(template: &Map<String, Value>, slug: &str, priority: i64) -> Value {
    let mut entry = template.clone();
    for field in OPENAI_ONLY {
        entry.remove(field);
    }
    let lower = slug.to_ascii_lowercase();
    let family = FAMILIES
        .iter()
        .find(|family| lower.starts_with(family.prefix))
        .unwrap_or(&UNKNOWN);
    let efforts: Vec<Value> = family.efforts.iter().map(|name| effort(name)).collect();
    for (field, value) in [
        ("slug", json!(slug)),
        ("display_name", json!(slug)),
        ("description", json!("通过 OpenVAIR 使用")),
        ("visibility", json!("list")),
        ("priority", json!(priority)),
        ("context_window", json!(family.context)),
        ("max_context_window", json!(family.context)),
        ("supported_reasoning_levels", json!(efforts)),
        ("default_reasoning_level", json!(family.default_effort)),
        ("shell_type", json!("shell_command")),
        ("tool_mode", Value::Null),
        ("use_responses_lite", json!(false)),
        ("input_modalities", json!(["text"])),
        ("supports_image_detail_original", json!(false)),
        // Turns on Codex's `tool_search`, whose results DeepSeek rejects.
        ("supports_search_tool", json!(false)),
        ("web_search_tool_type", json!("text")),
        ("experimental_supported_tools", json!([])),
    ] {
        entry.insert(field.into(), value);
    }
    entry.remove("multi_agent_reasoning_effort");
    Value::Object(entry)
}

fn models(file: &Map<String, Value>) -> Vec<Map<String, Value>> {
    file.get("models")
        .and_then(Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(|model| model.as_object().cloned())
                .filter(|model| model.get("slug").and_then(Value::as_str).is_some())
                .collect()
        })
        .unwrap_or_default()
}

fn slug(model: &Map<String, Value>) -> &str {
    model
        .get("slug")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// The cache's identity, so a newer download is noticed.
fn fingerprint(cache: &Map<String, Value>) -> Value {
    json!({
        "etag": cache.get("etag"),
        "fetched_at": cache.get("fetched_at"),
        "client_version": cache.get("client_version"),
    })
}

/// Codex's own models, from its download and from the newest Codex
/// installed. The download stops at the last ChatGPT sign-in, so where both
/// list a model the newer release's entry is kept; on a tie, the download,
/// which is ChatGPT's list for this account. Also returns what identifies
/// both, so a change in either is noticed.
fn known(dir: &Path) -> Result<(Vec<Map<String, Value>>, Value)> {
    let cache = read_json(&dir.join(CACHE))?;
    let bundled = codex_bundled::find(dir.parent().unwrap_or(dir));
    let sources = json!({
        "cache": fingerprint(&cache),
        "bundled": bundled.as_ref().map(|bundled| &bundled.version),
    });
    let cached = models(&cache);
    let Some(bundled) = bundled else {
        return Ok((cached, sources));
    };
    let shipped = models(&bundled.catalog);
    let (first, second) = match cache.get("client_version").and_then(Value::as_str) {
        Some(version) if !codex_bundled::newer(&bundled.version, version) => (cached, shipped),
        _ => (shipped, cached),
    };
    let names: BTreeSet<String> = first.iter().map(|model| slug(model).to_owned()).collect();
    let mut list = first;
    list.extend(
        second
            .into_iter()
            .filter(|model| !names.contains(slug(model))),
    );
    Ok((list, sources))
}

/// Builds the catalog for `wanted`. Returns `None` when Codex already knows
/// every model or has no model to copy from, in which case no catalog is
/// needed or none can be made.
pub fn build(dir: &Path, wanted: &BTreeSet<String>) -> Result<Option<Map<String, Value>>> {
    let (known, sources) = known(dir)?;
    let names: BTreeSet<&str> = known.iter().map(slug).collect();
    let extra: Vec<&String> = wanted
        .iter()
        .filter(|model| !names.contains(model.as_str()) && !is_openai(model))
        .collect();
    if extra.is_empty() {
        return Ok(None);
    }
    // The model Codex lists first is the one it is tuned for now.
    let Some(template) = known
        .iter()
        .filter(|model| model.get("visibility").and_then(Value::as_str) == Some("list"))
        .min_by_key(|model| {
            model
                .get("priority")
                .and_then(Value::as_i64)
                .unwrap_or(i64::MAX)
        })
    else {
        return Ok(None);
    };
    let last = known
        .iter()
        .filter_map(|model| model.get("priority").and_then(Value::as_i64))
        .max()
        .unwrap_or(0);
    let mut list: Vec<Value> = known.iter().cloned().map(Value::Object).collect();
    list.extend(
        extra
            .iter()
            .zip(1..)
            .map(|(model, offset)| copy(template, model, last + offset)),
    );
    let mut catalog = Map::new();
    catalog.insert("models".into(), Value::Array(list));
    catalog.insert(META.into(), json!({ "sources": sources, "models": extra }));
    Ok(Some(catalog))
}

/// Writes the catalog and returns its path, or removes an outdated one.
pub struct Written {
    /// The catalog to point Codex at, if the key needs one.
    pub path: Option<std::path::PathBuf>,
    pub changed: bool,
}

/// Writes the catalog, leaving an identical file untouched.
/// The models a catalog for `wanted` would add to Codex's own, or none
/// when no catalog would be written.
pub fn listed(dir: &Path, wanted: &BTreeSet<String>) -> Result<Vec<String>> {
    let Some(catalog) = build(dir, wanted)? else {
        return Ok(Vec::new());
    };
    let known: BTreeSet<String> = known(dir)?
        .0
        .iter()
        .map(|model| slug(model).to_owned())
        .collect();
    Ok(models(&catalog)
        .iter()
        .map(|model| slug(model).to_owned())
        .filter(|name| !known.contains(name))
        .collect())
}

pub fn write(dir: &Path, wanted: &BTreeSet<String>) -> Result<Written> {
    let path = dir.join(FILE);
    match build(dir, wanted)? {
        Some(catalog) => {
            let changed = read_json(&path).ok().as_ref() != Some(&catalog);
            if changed {
                write_json(&path, catalog)?;
            }
            Ok(Written {
                path: Some(path),
                changed,
            })
        }
        None => {
            let changed = path.exists();
            remove(dir);
            Ok(Written {
                path: None,
                changed,
            })
        }
    }
}

pub fn remove(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(FILE));
}

/// Rebuilds the catalog after Codex downloads a newer one or is upgraded, so
/// its own models keep the prompts of the installed version. Returns whether
/// the file changed.
pub fn refresh(dir: &Path) -> Result<bool> {
    let path = dir.join(FILE);
    if !path.exists() {
        return Ok(false);
    }
    let current = read_json(&path)?;
    let Some(meta) = current.get(META) else {
        return Ok(false);
    };
    if meta.get("sources") == Some(&known(dir)?.1) {
        return Ok(false);
    }
    let wanted: BTreeSet<String> = meta
        .get("models")
        .and_then(Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();
    // Without a model to copy from the old file is better than none.
    match build(dir, &wanted)? {
        Some(catalog) => {
            write_json(&path, catalog)?;
            Ok(true)
        }
        None => Ok(false),
    }
}
