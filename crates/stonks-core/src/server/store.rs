//! JSON persistence for the selected server. The host app picks the file
//! location (Tauri's config dir today, shared with the GPUI app).

use std::path::{Path, PathBuf};

use crate::error::{CommandError, ErrorCode};

use super::endpoints::validate_selection;
use super::models::ServerSelection;

pub fn read_selection(path: &Path) -> Result<Option<ServerSelection>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes =
        std::fs::read(path).map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    let selection: ServerSelection = serde_json::from_slice(&bytes)
        .map_err(|error| format!("The saved server configuration is invalid: {error}"))?;
    validate_selection(&selection)
        .map_err(|_| "The saved server configuration contains an insecure endpoint.".to_owned())?;
    Ok(Some(selection))
}

pub fn save_selection(path: &Path, selection: &ServerSelection) -> Result<(), CommandError> {
    validate_selection(selection)?;
    let unavailable = || CommandError::new(ErrorCode::Unavailable, "Could not save the server selection.");
    let parent: PathBuf = path.parent().ok_or_else(unavailable)?.to_owned();
    std::fs::create_dir_all(parent).map_err(|_| unavailable())?;
    let bytes = serde_json::to_vec_pretty(selection).map_err(|_| {
        CommandError::new(
            ErrorCode::InvalidResponse,
            "The server selection could not be encoded.",
        )
    })?;
    std::fs::write(path, bytes).map_err(|_| unavailable())
}
