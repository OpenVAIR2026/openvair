//! Agent adapters. Each one knows where its agent keeps configuration and
//! changes only the fields OpenVAIR manages, recording the values it replaced so
//! they can be put back.
//!
//! Adding an agent takes an `AgentId` variant, a module with an `Adapter`
//! and a line in `ALL`. Agents with a list of named providers get an `openvair`
//! entry of their own; the rest have OpenVAIR's values written over theirs.
mod claude_code;
mod codex;
mod codex_bundled;
mod codex_models;
mod deepseek_harness;
mod detect;
mod gemini_cli;
mod kimi_cli;
mod opencode;
mod qwen_code;
mod zcode;

use crate::{Result, mask_secret};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentId {
    Codex,
    ClaudeCode,
    GeminiCli,
    QwenCode,
    KimiCli,
    #[serde(rename = "opencode")]
    OpenCode,
    #[serde(rename = "zcode")]
    ZCode,
    #[serde(rename = "dsh")]
    DeepSeekHarness,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    #[serde(rename = "openai")]
    OpenAi,
    Anthropic,
    Gemini,
}

impl Protocol {
    /// The protocol name the server uses for model lists.
    pub fn server_name(self) -> &'static str {
        match self {
            Protocol::OpenAi => "openai",
            Protocol::Anthropic => "anthropic",
            Protocol::Gemini => "gemini",
        }
    }
}

/// The order agents are listed in.
pub const ALL: [AgentId; 8] = [
    AgentId::Codex,
    AgentId::ClaudeCode,
    AgentId::GeminiCli,
    AgentId::QwenCode,
    AgentId::KimiCli,
    AgentId::OpenCode,
    AgentId::ZCode,
    AgentId::DeepSeekHarness,
];

/// What an agent's configuration currently says about OpenVAIR.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Current {
    pub base_url: Option<String>,
    #[serde(skip)]
    pub secret: Option<String>,
    pub model: Option<String>,
}

/// The settings OpenVAIR writes.
pub struct Settings<'a> {
    pub server: &'a Url,
    pub secret: &'a str,
    pub model: &'a str,
    /// The key's models for the agent's protocol, for agents that list them.
    pub models: &'a [String],
    /// Claude Code's model for background work, when its aliases change;
    /// `None` uses `model`.
    pub background_model: Option<&'a str>,
    /// For agents that speak more than one protocol; `None` is the agent's
    /// first.
    pub protocol: Option<Protocol>,
}

/// A line of the change preview; secrets are masked.
#[derive(Debug, Clone, Serialize)]
pub struct Change {
    pub field: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

/// The values OpenVAIR replaced, by field, restored on disconnect.
pub type Prior = BTreeMap<String, Option<Value>>;

/// What the interface shows about an agent, and where its files are.
pub struct Meta {
    pub name: &'static str,
    /// Two or three characters standing in for the agent's logo.
    pub glyph: &'static str,
    /// Where to learn how to install it.
    pub homepage: &'static str,
    /// The protocols it can use; the first is the default.
    pub protocols: &'static [Protocol],
    /// Command names that show it is installed.
    pub binaries: &'static [&'static str],
    /// Files only running or installing the agent creates, relative to the
    /// home folder (or absolute, for an app bundle), for when its command
    /// cannot be found. Never anything in OpenVAIR's own `files`.
    pub traces: &'static [&'static str],
    /// Its configuration folder, relative to the home folder.
    pub dir: &'static str,
    /// The files OpenVAIR may change, relative to `dir`, in the order shown.
    pub files: &'static [&'static str],
    /// How the preview names the base URL, key and model.
    pub fields: [&'static str; 3],
    /// Whether its address may go through bili, which a new key keeps.
    pub bili: bool,
    /// What to do once the file is written; `None` means reopening the agent.
    pub after_write: Option<&'static str>,
}

/// One agent's way of reading and writing its configuration. `files` are
/// `Meta::files` resolved against the home folder.
pub trait Adapter: Sync {
    fn meta(&self) -> &'static Meta;

    fn read(&self, files: &[PathBuf]) -> Result<Current>;

    /// Writes OpenVAIR's settings and returns the values they replaced.
    fn apply(&self, files: &[PathBuf], settings: &Settings) -> Result<Prior>;

    /// Removes OpenVAIR's settings, putting back what was there before.
    fn remove(&self, files: &[PathBuf], prior: &Prior) -> Result<()>;

    /// The base URL written for `protocol`, when the agent's SDK differs from
    /// `protocol_url`.
    fn base_url(&self, protocol: Protocol, server: &Url) -> String {
        protocol_url(protocol, server)
    }

    /// Preview lines beyond the base URL, key and model.
    fn preview(&self, _files: &[PathBuf], _settings: &Settings) -> Result<Vec<Change>> {
        Ok(Vec::new())
    }

    /// See `AgentId::sync_models`.
    fn sync_models(&self, _files: &[PathBuf], _model: &str, _models: &[String]) -> Result<bool> {
        Ok(false)
    }

    /// See `AgentId::asks_background_model`.
    fn asks_background_model(&self, _files: &[PathBuf], _models: &[String]) -> Result<bool> {
        Ok(false)
    }

    /// See `AgentId::notice`; `dir` is the configuration folder.
    fn notice(&self, _dir: &Path) -> Option<&'static str> {
        None
    }

    /// See `AgentId::refresh`; `dir` is the configuration folder.
    fn refresh(&self, _dir: &Path) -> Result<bool> {
        Ok(false)
    }
}

impl AgentId {
    fn adapter(self) -> &'static dyn Adapter {
        match self {
            AgentId::Codex => &codex::Codex,
            AgentId::ClaudeCode => &claude_code::ClaudeCode,
            AgentId::GeminiCli => &gemini_cli::GeminiCli,
            AgentId::QwenCode => &qwen_code::QwenCode,
            AgentId::KimiCli => &kimi_cli::KimiCli,
            AgentId::OpenCode => &opencode::OpenCode,
            AgentId::ZCode => &zcode::ZCode,
            AgentId::DeepSeekHarness => &deepseek_harness::DeepSeekHarness,
        }
    }

    pub fn meta(self) -> &'static Meta {
        self.adapter().meta()
    }

    pub fn name(self) -> &'static str {
        self.meta().name
    }

    /// The protocol the agent uses unless told otherwise.
    pub fn protocol(self) -> Protocol {
        self.meta().protocols[0]
    }

    /// `requested` if the agent can use it, otherwise its default.
    pub fn protocol_or_default(self, requested: Option<Protocol>) -> Protocol {
        requested
            .filter(|protocol| self.meta().protocols.contains(protocol))
            .unwrap_or_else(|| self.protocol())
    }

    fn config_dir(self, home: &Path) -> PathBuf {
        home.join(self.meta().dir)
    }

    /// The files OpenVAIR may change, in the order shown to the user.
    pub fn files(self, home: &Path) -> Vec<PathBuf> {
        let dir = self.config_dir(home);
        self.meta()
            .files
            .iter()
            .map(|file| dir.join(file))
            .collect()
    }

    /// Installed means the agent's configuration folder exists or its command
    /// is found. Apps started from the Dock do not inherit the shell's PATH,
    /// so common install folders are checked too.
    pub fn installed(self, home: &Path) -> bool {
        let meta = self.meta();
        detect::installed(home, meta.binaries, meta.traces)
    }

    pub fn read(self, home: &Path) -> Result<Current> {
        self.adapter().read(&self.files(home))
    }

    /// Something about the agent's own setup that OpenVAIR leaves alone but the
    /// user should know, as a code the interface explains.
    pub fn notice(self, home: &Path) -> Option<&'static str> {
        self.adapter().notice(&self.config_dir(home))
    }

    /// Writes OpenVAIR's settings and returns the values they replaced.
    pub fn apply(self, home: &Path, settings: &Settings) -> Result<Prior> {
        self.adapter().apply(&self.files(home), settings)
    }

    /// Removes OpenVAIR's settings, putting back what was there before.
    pub fn remove(self, home: &Path, prior: &Prior) -> Result<()> {
        self.adapter().remove(&self.files(home), prior)
    }

    /// Updates the model lists OpenVAIR wrote for the agent to the key's current
    /// models, leaving everything else as it is. Returns whether anything
    /// changed.
    pub fn sync_models(self, home: &Path, model: &str, models: &[String]) -> Result<bool> {
        self.adapter().sync_models(&self.files(home), model, models)
    }

    /// Keeps files OpenVAIR generated for the agent in step with the agent's
    /// own updates. Returns whether anything changed.
    pub fn refresh(self, home: &Path) -> Result<bool> {
        self.adapter().refresh(&self.config_dir(home))
    }

    /// Whether connecting with these models changes the model Claude Code
    /// uses for background work, so the user should choose it.
    pub fn asks_background_model(self, home: &Path, models: &[String]) -> Result<bool> {
        self.adapter()
            .asks_background_model(&self.files(home), models)
    }

    /// Previews a change without writing. `settings.secret` holds only the
    /// key's prefix, which is shown masked.
    pub fn preview(self, home: &Path, settings: &Settings) -> Result<Vec<Change>> {
        let meta = self.meta();
        let current = self.read(home)?;
        let [base_field, key_field, model_field] = meta.fields;
        let url = self
            .adapter()
            .base_url(self.protocol_or_default(settings.protocol), settings.server);
        let mut changes = vec![
            Change {
                field: base_field.into(),
                after: Some(if meta.bili {
                    crate::bili::keep_wrapping(current.base_url.as_deref(), &url)
                } else {
                    url
                }),
                before: current.base_url.clone(),
            },
            Change {
                field: key_field.into(),
                before: current.secret.as_deref().map(mask_secret),
                after: Some(format!("{}…••••", settings.secret)),
            },
            Change {
                field: model_field.into(),
                before: current.model.clone(),
                after: Some(settings.model.to_owned()),
            },
        ];
        changes.extend(self.adapter().preview(&self.files(home), settings)?);
        Ok(changes)
    }
}

/// The agent's base URL for its default protocol.
pub fn base_url(agent: AgentId, server: &Url) -> String {
    protocol_url(agent.protocol(), server)
}

/// Each SDK appends its own path: OpenAI clients expect `/v1` in the base URL,
/// the Anthropic and Gemini SDKs add theirs.
pub fn protocol_url(protocol: Protocol, server: &Url) -> String {
    let root = server.as_str().trim_end_matches('/');
    match protocol {
        Protocol::OpenAi => format!("{root}/v1"),
        Protocol::Anthropic | Protocol::Gemini => root.to_owned(),
    }
}

/// The models an agent lists: the chosen one first, then the key's, once each.
fn listed_models(model: &str, models: &[String]) -> Vec<String> {
    let mut listed: Vec<String> = Vec::new();
    for name in std::iter::once(model).chain(models.iter().map(String::as_str)) {
        if !name.is_empty() && !listed.iter().any(|seen| seen == name) {
            listed.push(name.to_owned());
        }
    }
    listed
}

/// `map[key]` as an object, replacing anything else found there.
fn object_mut<'a>(
    map: &'a mut serde_json::Map<String, Value>,
    key: &str,
) -> &'a mut serde_json::Map<String, Value> {
    let entry = map
        .entry(key)
        .or_insert_with(|| Value::Object(Default::default()));
    if !entry.is_object() {
        *entry = Value::Object(Default::default());
    }
    entry.as_object_mut().expect("an object")
}

/// Follows `path` through nested objects.
fn lookup<'a>(map: &'a serde_json::Map<String, Value>, path: &[&str]) -> Option<&'a Value> {
    let (first, rest) = path.split_first()?;
    rest.iter()
        .try_fold(map.get(*first)?, |value, key| value.get(key))
}

fn text(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(ToOwned::to_owned)
}

/// Records `field`'s old value once, so later writes keep the original.
fn remember(prior: &mut BTreeMap<String, Option<Value>>, field: &str, value: Option<Value>) {
    prior.entry(field.to_owned()).or_insert(value);
}

pub(crate) fn read_text(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(crate::CoreError::file(path, error)),
    }
}

pub(crate) fn write_text(path: &Path, text: &str) -> Result<()> {
    crate::state::write_private(path, text.as_bytes())
        .map_err(|error| crate::CoreError::file(path, error))
}

/// Reads a JSON object file, treating a missing or empty file as `{}`.
pub(crate) fn read_json(path: &Path) -> Result<serde_json::Map<String, Value>> {
    match read_text(path)? {
        None => Ok(Default::default()),
        Some(text) if text.trim().is_empty() => Ok(Default::default()),
        Some(text) => match serde_json::from_str(&text) {
            Ok(Value::Object(map)) => Ok(map),
            _ => Err(crate::CoreError::InvalidConfig(path.display().to_string())),
        },
    }
}

pub(crate) fn write_json(path: &Path, map: serde_json::Map<String, Value>) -> Result<()> {
    let mut text = serde_json::to_string_pretty(&Value::Object(map))
        .map_err(|error| crate::CoreError::Other(error.to_string()))?;
    text.push('\n');
    write_text(path, &text)
}

#[cfg(test)]
mod tests;
