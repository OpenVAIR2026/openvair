//! The build edition. The hosted service (VAIR) builds with the `cloud`
//! feature; without it the binary is OpenVAIR, the self-hosted edition: one
//! account whose owner set up the instance, no sign-up and no features that
//! hand keys between accounts.
//!
//! Edition code is gated with `if CLOUD`, not `#[cfg]`, so both editions
//! compile, lint and test the same code.

pub const CLOUD: bool = cfg!(feature = "cloud");

/// The product name shown in emails and messages.
pub const PRODUCT: &str = if CLOUD { "VAIR" } else { "OpenVAIR" };
