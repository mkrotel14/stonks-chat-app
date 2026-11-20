//! Concrete `SessionAccess`: server selection from the JSON config file the
//! host app points at, session tokens from the OS keyring. The Tauri shell
//! and the GPUI app share the same config file and keyring entries, so a
//! session survives across both during the migration.

use std::path::PathBuf;

use crate::auth::credentials::load_token;
use crate::http::{ContextError, SessionAccess};
use crate::server::{self, ServerSelection};

pub struct SessionStore {
    config_path: PathBuf,
}

impl SessionStore {
    pub fn new(config_path: PathBuf) -> Self {
        Self { config_path }
    }
}

impl SessionAccess for SessionStore {
    fn server_selection(&self) -> Result<Option<ServerSelection>, ContextError> {
        server::read_selection(&self.config_path)
            .map_err(|_| ContextError::ConfigurationUnavailable)
    }

    fn token(&self, server: &ServerSelection) -> Result<Option<String>, ContextError> {
        load_token(server).map_err(|_| ContextError::SecureStorage)
    }
}
