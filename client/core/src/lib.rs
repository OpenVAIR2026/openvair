//! OpenVAIR Desktop core: keeps the user's OpenVAIR keys, reads their status
//! from the server, and writes them into local agent configurations. It has no GUI
//! dependencies so a command-line client can reuse it.
pub mod agents;
pub mod api;
pub mod bili;
pub mod credentials;
pub mod error;
pub mod probe;
pub mod state;

pub use error::{CoreError, Result};

/// Shows only enough of a secret to recognize it: `openvair_Ab3x…••••`.
pub fn mask_secret(secret: &str) -> String {
    let visible: String = secret.chars().take(9).collect();
    format!("{visible}…••••")
}
