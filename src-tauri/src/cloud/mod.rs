//! Cloud transcription providers, used instead of a local model when the
//! person picks one in Settings > Models.
//!
//! - `keychain` keeps provider API keys in the operating system's credential
//!   store, never in the settings file.
//! - `voxtral` sends a finished recording to Mistral Voxtral and returns its
//!   text.

pub mod keychain;
pub mod voxtral;
