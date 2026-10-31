//! What OpenVAIR Desktop remembers locally: the saved keys, and for each agent
//! which key and model it wrote and the values it replaced. Key secrets live in
//! the credential store, never here.
use crate::{
    CoreError, Result,
    agents::{AgentId, Protocol},
    probe::TestResult,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connection {
    /// The server the key belongs to, kept so the agent can still be tested
    /// and recognized if the key is later forgotten.
    #[serde(default)]
    pub server: String,
    pub key_id: String,
    pub key_name: String,
    pub key_prefix: String,
    pub model: String,
    pub connected_at: DateTime<Utc>,
    /// Values replaced on first connect, restored on disconnect.
    pub prior: BTreeMap<String, Option<Value>>,
    /// Files saved before the latest write, for undo.
    pub last_backups: Vec<Backup>,
    pub last_test: Option<TestResult>,
    /// The connection this one replaced, restored by undo.
    #[serde(default)]
    pub previous: Option<Box<Connection>>,
    /// Claude Code's background model chosen on connect, if it was asked.
    #[serde(default)]
    pub background_model: Option<String>,
    /// The key's models when the agent's lists were last written.
    #[serde(default)]
    pub models: Vec<String>,
    /// The latest background update of the agent's model lists.
    #[serde(default)]
    pub models_update: Option<ModelsUpdate>,
    /// The protocol chosen for an agent that speaks several; `None` is its
    /// default.
    #[serde(default)]
    pub protocol: Option<Protocol>,
}

impl Connection {
    /// Records the key's models, noting what changed since the last write.
    pub fn record_models(&mut self, models: Vec<String>) {
        if models == self.models {
            return;
        }
        // Connections from before a copy was kept have nothing to compare.
        if !self.models.is_empty() {
            let added = models.iter().filter(|model| !self.models.contains(model));
            let removed = self.models.iter().filter(|model| !models.contains(model));
            self.models_update = Some(ModelsUpdate {
                at: Utc::now(),
                added: added.cloned().collect(),
                removed: removed.cloned().collect(),
            });
        }
        self.models = models;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsUpdate {
    pub at: DateTime<Utc>,
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Backup {
    pub original: PathBuf,
    /// `None` when the file did not exist before the write.
    pub copy: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalState {
    /// The one OpenVAIR server this computer uses, set by the first key.
    #[serde(default)]
    pub server: Option<String>,
    #[serde(default)]
    pub keys: Vec<SavedKey>,
    /// The key new agent connections start from.
    #[serde(default)]
    pub default_key: Option<String>,
    #[serde(default)]
    pub connections: BTreeMap<AgentId, Connection>,
    /// Whether connected agents' model lists follow their keys.
    #[serde(default = "on")]
    pub sync_models: bool,
}

impl Default for LocalState {
    fn default() -> Self {
        LocalState {
            server: None,
            keys: Vec::new(),
            default_key: None,
            connections: BTreeMap::new(),
            sync_models: true,
        }
    }
}

fn on() -> bool {
    true
}

/// A key the user added, identified by its id on its server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedKey {
    pub id: String,
    /// Always the state's server for now; kept per key so several servers
    /// can be allowed later without moving data.
    pub server: String,
    /// The user's own label; `None` shows the key's name on the server.
    #[serde(default)]
    pub label: Option<String>,
    pub key_name: String,
    pub key_prefix: String,
    pub added_at: DateTime<Utc>,
}

impl SavedKey {
    pub fn name(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.key_name)
    }
}

impl LocalState {
    pub fn key(&self, id: &str) -> Option<&SavedKey> {
        self.keys.iter().find(|key| key.id == id)
    }

    /// The default key, falling back to the first one so there always is one
    /// while any key is saved.
    pub fn default_key(&self) -> Option<&SavedKey> {
        self.default_key
            .as_deref()
            .and_then(|id| self.key(id))
            .or_else(|| self.keys.first())
    }

    /// Agents currently written with the key.
    pub fn agents_using(&self, key_id: &str) -> Vec<AgentId> {
        self.connections
            .iter()
            .filter(|(_, connection)| connection.key_id == key_id)
            .map(|(agent, _)| *agent)
            .collect()
    }
}

/// `OPENVAIR_DATA_DIR` lets a development build run next to the installed
/// app without sharing its state.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("OPENVAIR_DATA_DIR").filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("OpenVAIR")
}

pub fn backups_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("backups")
}

impl LocalState {
    fn path(data_dir: &Path) -> PathBuf {
        data_dir.join("state.json")
    }

    pub fn load(data_dir: &Path) -> LocalState {
        std::fs::read(Self::path(data_dir))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, data_dir: &Path) -> Result<()> {
        let path = Self::path(data_dir);
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| CoreError::Other(e.to_string()))?;
        write_private(&path, &bytes).map_err(|error| CoreError::file(&path, error))
    }
}

/// Writes a file readable only by the user, via a temporary file and rename so
/// an agent never reads a half-written configuration.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("openvair-tmp");
    let _ = std::fs::remove_file(&temporary);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    // Created private, so the secret is never readable by others in between.
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    std::io::Write::write_all(&mut options.open(&temporary)?, bytes)?;
    std::fs::rename(&temporary, path)
}

/// Copies each file aside before OpenVAIR Desktop changes it.
pub fn back_up(files: &[PathBuf], data_dir: &Path, label: &str) -> Result<Vec<Backup>> {
    let directory = backups_dir(data_dir);
    let stamp = Utc::now().format("%Y%m%d-%H%M%S%.3f");
    files
        .iter()
        .map(|original| {
            if !original.exists() {
                return Ok(Backup {
                    original: original.clone(),
                    copy: None,
                });
            }
            let name = original
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "config".into());
            let copy = directory.join(format!("{label}-{stamp}-{name}"));
            let bytes =
                std::fs::read(original).map_err(|error| CoreError::file(original, error))?;
            write_private(&copy, &bytes).map_err(|error| CoreError::file(&copy, error))?;
            Ok(Backup {
                original: original.clone(),
                copy: Some(copy),
            })
        })
        .collect()
}

pub fn restore(backups: &[Backup]) -> Result<()> {
    for backup in backups {
        match &backup.copy {
            Some(copy) => {
                let bytes = std::fs::read(copy).map_err(|error| CoreError::file(copy, error))?;
                write_private(&backup.original, &bytes)
                    .map_err(|error| CoreError::file(&backup.original, error))?;
            }
            None => {
                let _ = std::fs::remove_file(&backup.original);
            }
        }
    }
    Ok(())
}
