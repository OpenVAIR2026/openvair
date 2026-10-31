//! DeepSeek Harness (`dsh`) composes `~/.dsh/cordis.patch.yml` over every
//! profile, the Desktop one included, and resolves API keys through
//! `~/.dsh/.env` among other sources. Its DeepSeek route speaks only the
//! Anthropic Messages protocol and accepts a compatible gateway as `baseURL`.
//!
//! OpenVAIR appends a marked block to the patch that replaces the `llm-deepseek`
//! and `agent-default-model` rows, and keeps the key in `.env` as
//! `OPENVAIR_API_KEY` so an existing `DEEPSEEK_API_KEY` is neither used nor
//! touched. Disconnecting drops the block, which lets the user's own rows and
//! the shipped defaults apply again.
use super::gemini_cli::{env_values, set_env};
use super::{
    Current, Meta, Prior, Protocol, Settings, listed_models, read_text, remember, write_text,
};
use crate::{CoreError, Result};
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf};

const KEY_VAR: &str = "OPENVAIR_API_KEY";
const BEGIN: &str = "# >>> OpenVAIR: managed by the OpenVAIR client, removed on disconnect";
const END: &str = "# <<< OpenVAIR";
/// An empty patch fails dsh's boot; an empty list is how it is switched off.
const EMPTY: &str = "[]\n";

/// Single-quoted YAML scalar.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

fn unquote(text: &str) -> String {
    let text = text.trim();
    match text.strip_prefix('\'').and_then(|t| t.strip_suffix('\'')) {
        Some(inner) => inner.replace("''", "'"),
        None => text.to_owned(),
    }
}

/// Splits the patch into the user's lines and OpenVAIR's block, refusing files
/// whose root is not a block list that the block can be appended to.
fn split(text: &str, path: &std::path::Path) -> Result<(Vec<String>, Vec<String>)> {
    let mut own = Vec::new();
    let mut block = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if line == BEGIN {
            inside = true;
        } else if inside && line == END {
            inside = false;
        } else if inside {
            block.push(line.to_owned());
        } else {
            own.push(line.to_owned());
        }
    }
    let content: Vec<&str> = own
        .iter()
        .map(|line| line.as_str())
        .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .collect();
    match content.first() {
        None => {}
        Some(&"[]") if content.len() == 1 => own.retain(|line| line.trim() != "[]"),
        Some(first) if *first == "-" || first.starts_with("- ") => {}
        Some(_) => {
            return Err(CoreError::Other(format!(
                "{} 不是 OpenVAIR 能追加的 YAML 列表，请用“接入 Agent”里的“手动接入”",
                path.display()
            )));
        }
    }
    Ok((own, block))
}

fn join(own: &[String], block: Option<Vec<String>>) -> String {
    let mut lines: Vec<String> = own.to_vec();
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    if let Some(block) = block {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(BEGIN.to_owned());
        lines.extend(block);
        lines.push(END.to_owned());
    }
    if lines
        .iter()
        .all(|line| line.trim().is_empty() || line.trim_start().starts_with('#'))
    {
        lines.push(EMPTY.trim_end().to_owned());
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// DeepSeek's own models keep the capabilities dsh ships for them.
fn model_rows(name: &str) -> Vec<String> {
    let mut rows = vec![format!("      - id: {}", quote(name))];
    match name {
        "deepseek-flash" => rows.extend([
            "        name: DeepSeek-V4.1-Flash".to_owned(),
            "        inputModalities: [text, image]".to_owned(),
            "        systemPromptUpdate: in-history".to_owned(),
            "        toolUpdate: addition-only".to_owned(),
        ]),
        "deepseek-v4-pro" => rows.push("        name: DeepSeek-V4-Pro".to_owned()),
        _ => {}
    }
    rows
}

fn block(base_url: &str, model: &str, models: &[String]) -> Vec<String> {
    let mut lines = vec![
        "- id: llm-deepseek".to_owned(),
        "  name: '@deepseek-ai/dsh-llm-deepseek-api-key'".to_owned(),
        "  config:".to_owned(),
        format!("    apiKeyEnv: {KEY_VAR}"),
        format!("    baseURL: {}", quote(base_url)),
        "    models:".to_owned(),
    ];
    for name in listed_models(model, models) {
        lines.extend(model_rows(&name));
    }
    lines.extend([
        "- id: agent-default-model".to_owned(),
        "  name: '@deepseek-ai/dsh-agent-default-model'".to_owned(),
        "  config:".to_owned(),
        "    provider: deepseek-official".to_owned(),
        format!("    model: {}", quote(model)),
    ]);
    lines
}

/// The value of `    <field>: ...` in OpenVAIR's own block.
fn field(block: &[String], name: &str) -> Option<String> {
    let prefix = format!("    {name}: ");
    block
        .iter()
        .find_map(|line| line.strip_prefix(&prefix))
        .map(unquote)
}

pub fn read(files: &[PathBuf]) -> Result<Current> {
    let text = read_text(&files[0])?.unwrap_or_default();
    let (_, block) = split(&text, &files[0])?;
    if block.is_empty() {
        return Ok(Current::default());
    }
    let env = env_values(&read_text(&files[1])?.unwrap_or_default());
    Ok(Current {
        base_url: field(&block, "baseURL"),
        secret: env.get(KEY_VAR).cloned(),
        model: field(&block, "model"),
    })
}

pub fn apply(files: &[PathBuf], settings: &Settings) -> Result<Prior> {
    let text = read_text(&files[0])?.unwrap_or_default();
    let (own, _) = split(&text, &files[0])?;
    let env = read_text(&files[1])?.unwrap_or_default();
    let mut prior = Prior::new();
    remember(
        &mut prior,
        KEY_VAR,
        env_values(&env).get(KEY_VAR).cloned().map(Value::String),
    );
    let url = super::protocol_url(Protocol::Anthropic, settings.server);
    let updates = BTreeMap::from([(KEY_VAR, Some(settings.secret.to_owned()))]);
    write_text(&files[1], &set_env(&env, &updates))?;
    write_text(
        &files[0],
        &join(&own, Some(block(&url, settings.model, settings.models))),
    )?;
    Ok(prior)
}

pub fn remove(files: &[PathBuf], prior: &Prior) -> Result<()> {
    if files[0].exists() {
        let text = read_text(&files[0])?.unwrap_or_default();
        let (own, _) = split(&text, &files[0])?;
        write_text(&files[0], &join(&own, None))?;
    }
    if files[1].exists() {
        let env = read_text(&files[1])?.unwrap_or_default();
        let old = prior
            .get(KEY_VAR)
            .cloned()
            .flatten()
            .and_then(|v| v.as_str().map(ToOwned::to_owned));
        write_text(&files[1], &set_env(&env, &BTreeMap::from([(KEY_VAR, old)])))?;
    }
    Ok(())
}

pub fn sync_models(files: &[PathBuf], model: &str, models: &[String]) -> Result<bool> {
    let text = read_text(&files[0])?.unwrap_or_default();
    let (own, old) = split(&text, &files[0])?;
    let Some(url) = field(&old, "baseURL") else {
        return Ok(false);
    };
    let after = join(&own, Some(block(&url, model, models)));
    if after == text {
        return Ok(false);
    }
    write_text(&files[0], &after)?;
    Ok(true)
}

pub struct DeepSeekHarness;

impl super::Adapter for DeepSeekHarness {
    fn meta(&self) -> &'static Meta {
        &Meta {
            name: "DeepSeek Harness",
            glyph: "DS",
            homepage: "https://www.npmjs.com/package/@deepseek-ai/dsh",
            protocols: &[Protocol::Anthropic],
            binaries: &["dsh"],
            traces: &[".dsh/profiles"],
            dir: ".dsh",
            files: &["cordis.patch.yml", ".env"],
            fields: [
                "llm-deepseek.baseURL",
                "OPENVAIR_API_KEY",
                "agent-default-model.model",
            ],
            bili: false,
            after_write: Some(
                "重新启动 dsh 后生效。OpenVAIR 的设置写在 ~/.dsh/cordis.patch.yml，优先于 dsh 设置页里的 DeepSeek 模型配置，断开后恢复。",
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
