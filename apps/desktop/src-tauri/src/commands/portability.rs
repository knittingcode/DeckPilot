//! Encrypted configuration export and import.
//!
//! The file paths come from native save/open dialogs in the UI. The export
//! password is separate from the database password and only used to derive
//! the archive key. An import is previewed first; the decrypted content is
//! kept in memory until the user applies or cancels it.

use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use tauri::State;

use deckpilot_core::ProfileId;
use deckpilot_portability::archive::MAX_ARCHIVE_BYTES;
use deckpilot_portability::import::{self, ImportPreview};
use deckpilot_portability::{
    ExportOptions, ImportOptions, ImportSummary, Payload, PortabilityError, payload,
};
use deckpilot_storage::crypto::{KdfParams, Password};
use deckpilot_storage::{ConfigStore as _, StorageError};

use crate::error::{CommandError, CommandResult};
use crate::state::{APP_VERSION, AppState};

/// The decrypted export between preview and apply.
#[derive(Debug, Default)]
pub struct PendingImport(Mutex<Option<(u32, Payload)>>);

impl PendingImport {
    fn take(&self, token: u32) -> Option<Payload> {
        let mut pending = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        match pending.take() {
            Some((stored, payload)) if stored == token => Some(payload),
            other => {
                *pending = other;
                None
            }
        }
    }

    /// A random token (32 bits, exact in JavaScript numbers) ties the apply
    /// call to the preview the user saw.
    fn put(&self, payload: Payload) -> u32 {
        let mut bytes = [0u8; 4];
        let _ = deckpilot_storage::crypto::random_bytes(&mut bytes);
        let token = u32::from_le_bytes(bytes);
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some((token, payload));
        token
    }

    fn clear(&self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

impl From<PortabilityError> for CommandError {
    fn from(error: PortabilityError) -> Self {
        match error {
            PortabilityError::Storage(error) => error.into(),
            other => Self::Invalid(other.to_string()),
        }
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or_default()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    path: String,
    password: Password,
    confirmation: Password,
    /// `None`: all profiles.
    profiles: Option<Vec<ProfileId>>,
    include_module_settings: bool,
    /// Portable application settings (update preferences).
    #[serde(default)]
    include_app_settings: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    path: String,
    profiles: usize,
    modules: usize,
    bytes: usize,
}

/// Writes an encrypted export to `path` (chosen in a save dialog).
#[tauri::command]
pub fn export_configuration(
    state: State<'_, AppState>,
    request: ExportRequest,
) -> CommandResult<ExportResult> {
    request
        .password
        .validate_new(&request.confirmation)
        .map_err(StorageError::from)?;
    state.store()?;
    // Exports contain the whole configuration: a sensitive operation.
    state.take_authorization()?;
    let content = {
        let store = state.store()?;
        let modules = state.modules();
        payload::export(
            &store,
            &modules,
            request.profiles.as_deref(),
            ExportOptions {
                module_settings: request.include_module_settings,
                app_settings: request.include_app_settings,
            },
            now(),
            APP_VERSION,
        )?
    };
    let kdf = KdfParams::generate().map_err(StorageError::from)?;
    let bytes = payload::seal(&content, &request.password, kdf)?;
    let path = std::path::PathBuf::from(&request.path);
    // Written next to the target and renamed, so that a failure never leaves
    // a truncated export behind. Only encrypted bytes reach the disk.
    let temporary = path.with_extension("deckpilot.tmp");
    let written =
        std::fs::write(&temporary, &bytes).and_then(|()| std::fs::rename(&temporary, &path));
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(CommandError::Invalid(error.to_string()));
    }
    Ok(ExportResult {
        path: request.path,
        profiles: content.profiles.len(),
        modules: content.modules.len(),
        bytes: bytes.len(),
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewResult {
    token: u32,
    preview: ImportPreview,
}

/// Decrypts and validates an export and describes what importing it would
/// change. Nothing is written.
#[tauri::command]
pub fn preview_import(
    state: State<'_, AppState>,
    pending: State<'_, PendingImport>,
    path: String,
    password: Password,
) -> CommandResult<PreviewResult> {
    let size = std::fs::metadata(&path)
        .map_err(|e| CommandError::Invalid(e.to_string()))?
        .len();
    if usize::try_from(size).map_or(true, |size| size > MAX_ARCHIVE_BYTES) {
        return Err(PortabilityError::TooLarge.into());
    }
    let bytes = std::fs::read(&path).map_err(|e| CommandError::Invalid(e.to_string()))?;
    let content = payload::open(&bytes, &password)?;
    let warnings = import::validate(&content, &state.catalog)?;
    let preview = {
        let store = state.store()?;
        let modules = state.modules();
        import::preview(&content, &store, &modules, warnings)?
    };
    let token = pending.put(content);
    Ok(PreviewResult { token, preview })
}

/// Sent to every window after imported data was written.
pub const CONFIGURATION_CHANGED: &str = "deckpilot://configuration-changed";

/// Applies a previewed import in one transaction.
#[tauri::command]
pub fn apply_import<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    pending: State<'_, PendingImport>,
    token: u32,
    options: ImportOptions,
) -> CommandResult<ImportSummary> {
    let content = pending
        .take(token)
        .ok_or_else(|| CommandError::Invalid("preview the import again".into()))?;
    let summary = {
        let store = state.store()?;
        let (changes, summary) = {
            let modules = state.modules();
            import::plan(&content, &options, &store, &modules)?
        };
        store.apply_changes(&changes)?;
        let (modules, problems) = deckpilot_modules::configured(&store.module_states()?);
        for problem in problems {
            eprintln!("imported module state was not applied: {problem}");
        }
        *state.modules() = modules;
        summary
    };
    crate::agent::notify_reload(&state.paths);
    let _ = tauri::Emitter::emit(&app, CONFIGURATION_CHANGED, ());
    Ok(summary)
}

#[tauri::command]
pub fn cancel_import(pending: State<'_, PendingImport>) {
    pending.clear();
}
