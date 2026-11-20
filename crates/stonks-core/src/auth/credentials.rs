use keyring::Entry;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

use crate::{
    error::{CommandError, ErrorCode},
    server::ServerSelection,
};

const KEYRING_SERVICE: &str = "com.rdxlabs.stonks.session";
const STRUCTURED_CREDENTIAL_PREFIX: &str = "stonks-session-v1:";
static CREDENTIAL_LOCK: Mutex<()> = Mutex::new(());

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredSession {
    token: String,
    session_id: String,
}

struct SessionData {
    token: String,
    session_id: Option<String>,
}

fn decode_credential(value: String) -> Result<SessionData, String> {
    let Some(json) = value.strip_prefix(STRUCTURED_CREDENTIAL_PREFIX) else {
        return Ok(SessionData {
            token: value,
            session_id: None,
        });
    };
    let stored: StoredSession = serde_json::from_str(json)
        .map_err(|error| format!("Could not decode the saved session: {error}"))?;
    if stored.token.is_empty() || stored.session_id.trim().is_empty() {
        return Err("The saved session is incomplete.".into());
    }
    Ok(SessionData {
        token: stored.token,
        session_id: Some(stored.session_id),
    })
}

fn encode_credential(token: &str, session_id: &str) -> Result<String, CommandError> {
    let stored = serde_json::to_string(&StoredSession {
        token: token.to_owned(),
        session_id: session_id.to_owned(),
    })
    .map_err(|_| secure_storage_error())?;
    Ok(format!("{STRUCTURED_CREDENTIAL_PREFIX}{stored}"))
}

pub struct SessionCredential {
    entry: Entry,
}

impl SessionCredential {
    pub fn open(server: &ServerSelection) -> Result<Self, CommandError> {
        token_entry(server)
            .map(|entry| Self { entry })
            .map_err(|_| secure_storage_error())
    }

    fn load_data(&self) -> Result<Option<SessionData>, CommandError> {
        match self.entry.get_password() {
            Ok(value) => decode_credential(value)
                .map(Some)
                .map_err(|_| secure_storage_error()),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(secure_storage_error()),
        }
    }

    pub fn load(&self) -> Result<Option<String>, CommandError> {
        let _guard = CREDENTIAL_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(self.load_data()?.map(|session| session.token))
    }

    pub fn store(&self, token: &str, session_id: &str) -> Result<(), CommandError> {
        let _guard = CREDENTIAL_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.entry
            .set_password(&encode_credential(token, session_id)?)
            .map_err(|_| secure_storage_error())
    }

    pub fn delete(&self) -> Result<(), CommandError> {
        let _guard = CREDENTIAL_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match self.entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(secure_storage_error()),
        }
    }
}

fn token_entry(server: &ServerSelection) -> Result<Entry, String> {
    Entry::new(KEYRING_SERVICE, &server.endpoints.api)
        .map_err(|error| format!("Could not open secure session storage: {error}"))
}

fn load_session_data(server: &ServerSelection) -> Result<Option<SessionData>, String> {
    let _guard = CREDENTIAL_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match token_entry(server)?.get_password() {
        Ok(value) => decode_credential(value).map(Some),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(format!("Could not read the saved session: {error}")),
    }
}

pub fn load_token(server: &ServerSelection) -> Result<Option<String>, String> {
    Ok(load_session_data(server)?.map(|session| session.token))
}

pub fn load_session_id(server: &ServerSelection) -> Result<Option<String>, String> {
    Ok(load_session_data(server)?.and_then(|session| session.session_id))
}

pub(crate) fn session_id_for_token(server: &ServerSelection, token: &str) -> Option<String> {
    load_session_data(server)
        .ok()
        .flatten()
        .filter(|saved| saved.token == token)
        .and_then(|saved| saved.session_id)
}

pub fn delete_token_if_matches(
    server: &ServerSelection,
    expected_token: &str,
) -> Result<bool, String> {
    let _guard = CREDENTIAL_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = token_entry(server)?;
    let stored = match entry.get_password() {
        Ok(value) => decode_credential(value)?,
        Err(keyring::Error::NoEntry) => return Ok(false),
        Err(error) => return Err(format!("Could not read the saved session: {error}")),
    };
    if stored.token != expected_token {
        return Ok(false);
    }
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(true),
        Err(error) => Err(format!("Could not delete the saved session: {error}")),
    }
}

fn secure_storage_error() -> CommandError {
    CommandError::new(
        ErrorCode::SecureStorage,
        "Could not access secure session storage.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_credentials_persist_across_entries_and_delete() {
        let user = format!(
            "test-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
        );
        let entry = Entry::new("com.rdxlabs.stonks.session.tests", &user).unwrap();
        struct Cleanup(Entry);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = self.0.delete_credential();
            }
        }
        let cleanup = Cleanup(entry);
        cleanup.0.set_password("ephemeral-test-session").unwrap();
        let reopened = Entry::new("com.rdxlabs.stonks.session.tests", &user).unwrap();
        assert_eq!(reopened.get_password().unwrap(), "ephemeral-test-session");
        reopened.delete_credential().unwrap();
        assert!(matches!(
            cleanup.0.get_password(),
            Err(keyring::Error::NoEntry)
        ));
    }

    #[test]
    fn legacy_plain_tokens_remain_readable_with_unknown_session_identity() {
        let decoded = decode_credential("legacy-token".into()).unwrap();
        assert_eq!(decoded.token, "legacy-token");
        assert_eq!(decoded.session_id, None);
    }

    #[test]
    fn structured_credentials_round_trip_without_exposing_the_token_shape() {
        let encoded = encode_credential("secret-token", "session-id").unwrap();
        assert!(encoded.starts_with(STRUCTURED_CREDENTIAL_PREFIX));
        let decoded = decode_credential(encoded).unwrap();
        assert_eq!(decoded.token, "secret-token");
        assert_eq!(decoded.session_id.as_deref(), Some("session-id"));
    }

    #[test]
    fn malformed_structured_credentials_do_not_fall_back_to_tokens() {
        assert!(decode_credential(format!("{STRUCTURED_CREDENTIAL_PREFIX}{{bad")).is_err());
    }
}
