//! Settings for cloud transcription: which engine transcribes, and the
//! Mistral API key.
//!
//! The key is write-only from the frontend's side: it can be saved, removed,
//! and checked for presence, never read back.

use crate::cloud::keychain;
use crate::settings::{self, TranscriptionProvider};
use log::warn;
use tauri::AppHandle;

#[tauri::command]
#[specta::specta]
pub fn change_transcription_provider_setting(
    app: AppHandle,
    provider: TranscriptionProvider,
) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.transcription_provider = provider;
    settings::write_settings(&app, settings);
    Ok(())
}

/// Saves the Mistral API key in the OS credential store, replacing any saved
/// one. The key is trimmed first.
///
/// # Errors
/// The key is empty, longer than 256 characters, or has characters an API key
/// can't have; or the credential store refused it.
#[tauri::command]
#[specta::specta]
pub async fn set_voxtral_api_key(key: String) -> Result<(), String> {
    let key = keychain::normalize_api_key(&key)
        .map_err(|e| e.to_string())?
        .to_string();
    // The credential store can block (macOS may ask for access), so keep it
    // off the async runtime's workers.
    tokio::task::spawn_blocking(move || keychain::store_voxtral_api_key(&key))
        .await
        .map_err(|e| format!("credential store task failed: {e}"))?
        .map_err(|e| {
            warn!("Couldn't save the Mistral API key: {e}");
            e
        })
}

/// Removes the saved Mistral API key, if any.
///
/// # Errors
/// The credential store refused the deletion.
#[tauri::command]
#[specta::specta]
pub async fn clear_voxtral_api_key() -> Result<(), String> {
    tokio::task::spawn_blocking(keychain::delete_voxtral_api_key)
        .await
        .map_err(|e| format!("credential store task failed: {e}"))?
        .map_err(|e| {
            warn!("Couldn't remove the Mistral API key: {e}");
            e
        })
}

/// Whether a Mistral API key is saved. An unreadable credential store counts
/// as no key.
#[tauri::command]
#[specta::specta]
pub async fn has_voxtral_api_key() -> bool {
    match tokio::task::spawn_blocking(keychain::voxtral_api_key).await {
        Ok(Ok(key)) => key.is_some(),
        Ok(Err(e)) => {
            warn!("Couldn't read the credential store: {e}");
            false
        }
        Err(e) => {
            warn!("Credential store task failed: {e}");
            false
        }
    }
}
