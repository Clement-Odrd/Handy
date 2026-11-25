//! API keys for cloud transcription, kept in the operating system's
//! credential store: Keychain on macOS, Credential Manager on Windows,
//! Secret Service on Linux.
//!
//! A key goes in through [`store_voxtral_api_key`] and only comes out for the
//! request that needs it. It is never written to the settings file, never
//! sent back to the frontend, and never logged: errors here carry the store's
//! message, not the key.

use keyring::{Entry, Error as KeyringError};

/// The credential store's service name: the app identifier, so the entry is
/// recognisable in Keychain Access or Credential Manager.
const SERVICE: &str = "com.pais.handy";
const VOXTRAL_ACCOUNT: &str = "mistral-voxtral-api-key";

/// Longer than any provider's key; anything past it is a paste mistake.
pub const MAX_API_KEY_LEN: usize = 256;

/// Why a pasted key was refused before reaching the credential store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyValidationError {
    Empty,
    TooLong,
    /// Whitespace inside the key or a non-ASCII character: it can't be sent
    /// in an `Authorization` header, so it is a paste mistake.
    InvalidCharacters,
}

impl std::fmt::Display for KeyValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Empty => "The API key is empty",
            Self::TooLong => "The API key is too long",
            Self::InvalidCharacters => "The API key contains characters an API key can't have",
        })
    }
}

/// The key as it will be stored: surrounding whitespace trimmed, then
/// checked to be non-empty, at most [`MAX_API_KEY_LEN`] bytes, and printable
/// ASCII without spaces.
pub fn normalize_api_key(raw: &str) -> Result<&str, KeyValidationError> {
    let key = raw.trim();
    if key.is_empty() {
        return Err(KeyValidationError::Empty);
    }
    if key.len() > MAX_API_KEY_LEN {
        return Err(KeyValidationError::TooLong);
    }
    if !key.chars().all(|c| c.is_ascii_graphic()) {
        return Err(KeyValidationError::InvalidCharacters);
    }
    Ok(key)
}

fn voxtral_entry() -> Result<Entry, String> {
    Entry::new(SERVICE, VOXTRAL_ACCOUNT).map_err(describe)
}

/// Saves the Mistral API key, replacing any saved one. `key` must already
/// have passed [`normalize_api_key`].
///
/// # Errors
/// The credential store's message when it refuses the write (locked or
/// unavailable store, denied access).
pub fn store_voxtral_api_key(key: &str) -> Result<(), String> {
    voxtral_entry()?.set_password(key).map_err(describe)
}

/// Removes the saved Mistral API key. Removing a key that isn't there
/// succeeds.
///
/// # Errors
/// The credential store's message when it refuses the deletion.
pub fn delete_voxtral_api_key() -> Result<(), String> {
    match voxtral_entry()?.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(e) => Err(describe(e)),
    }
}

/// The saved Mistral API key, or `None` when none is saved.
///
/// # Errors
/// The credential store's message when it can't be read.
pub fn voxtral_api_key() -> Result<Option<String>, String> {
    match voxtral_entry()?.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(e) => Err(describe(e)),
    }
}

/// A credential store error, safe to log: `BadEncoding` holds the stored
/// bytes, so it is reduced to a fixed message.
fn describe(error: KeyringError) -> String {
    match error {
        KeyringError::BadEncoding(_) => "the stored API key isn't valid text".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(normalize_api_key("  abc123\n"), Ok("abc123"));
    }

    #[test]
    fn rejects_an_empty_or_blank_key() {
        assert_eq!(normalize_api_key(""), Err(KeyValidationError::Empty));
        assert_eq!(normalize_api_key(" \t\n"), Err(KeyValidationError::Empty));
    }

    #[test]
    fn rejects_a_key_past_the_length_bound() {
        let at_bound = "a".repeat(MAX_API_KEY_LEN);
        assert!(normalize_api_key(&at_bound).is_ok());
        let past_bound = "a".repeat(MAX_API_KEY_LEN + 1);
        assert_eq!(
            normalize_api_key(&past_bound),
            Err(KeyValidationError::TooLong)
        );
    }

    #[test]
    fn rejects_inner_whitespace_and_non_ascii() {
        assert_eq!(
            normalize_api_key("abc def"),
            Err(KeyValidationError::InvalidCharacters)
        );
        assert_eq!(
            normalize_api_key("clé"),
            Err(KeyValidationError::InvalidCharacters)
        );
    }

    #[test]
    fn bad_encoding_never_echoes_the_stored_bytes() {
        let message = describe(KeyringError::BadEncoding(b"secret-key".to_vec()));
        assert!(!message.contains("secret"));
    }
}
