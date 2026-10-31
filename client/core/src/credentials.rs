//! Saved OpenVAIR keys live in the OS credential store (Windows Credential
//! Manager, Secret Service on Linux). On macOS, and wherever no store is
//! available, it is a file only the user can read.
use crate::{CoreError, Result};
use std::path::{Path, PathBuf};

const SERVICE: &str = "OpenVAIR Desktop";

pub struct Credentials {
    account: String,
    fallback: PathBuf,
}

impl Credentials {
    /// One secret per saved key, named by the key's id on its server.
    pub fn for_key(key_id: &str, data_dir: &Path) -> Self {
        Self::named(format!("key {key_id}"), data_dir)
    }

    fn named(account: String, data_dir: &Path) -> Self {
        let file = account.replace(['/', ':', ' '], "_");
        Self {
            account,
            fallback: data_dir.join(format!("token-{file}")),
        }
    }

    /// macOS ties keychain access to the code signature. Without a Developer
    /// ID that changes with every update and rebuild, so each one would ask
    /// for the keychain password again; the agents' own config files hold
    /// keys in plain text anyway.
    fn entry(&self) -> Option<keyring::Entry> {
        if cfg!(any(debug_assertions, target_os = "macos")) {
            return None;
        }
        keyring::Entry::new(SERVICE, &self.account).ok()
    }

    pub fn load(&self) -> Option<String> {
        if let Some(token) = self.entry().and_then(|entry| entry.get_password().ok()) {
            return Some(token);
        }
        std::fs::read_to_string(&self.fallback)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    }

    pub fn save(&self, token: &str) -> Result<()> {
        if self
            .entry()
            .is_some_and(|entry| entry.set_password(token).is_ok())
        {
            let _ = std::fs::remove_file(&self.fallback);
            return Ok(());
        }
        crate::state::write_private(&self.fallback, token.as_bytes())
            .map_err(|error| CoreError::file(&self.fallback, error))
    }

    pub fn clear(&self) {
        if let Some(entry) = self.entry() {
            let _ = entry.delete_credential();
        }
        let _ = std::fs::remove_file(&self.fallback);
    }
}
