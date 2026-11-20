//! Stable, user-safe command failures. Native diagnostics do not belong in this payload.
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidInput,
    NotConfigured,
    AuthenticationFailed,
    Unavailable,
    InvalidResponse,
    SecureStorage,
    NotConnected,
    NotFound,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct CommandError {
    code: ErrorCode,
    message: &'static str,
}

impl CommandError {
    pub const fn new(code: ErrorCode, message: &'static str) -> Self {
        Self { code, message }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for CommandError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_errors_have_only_stable_codes_and_safe_messages() {
        for (code, wire_code) in [
            (ErrorCode::InvalidInput, "invalid_input"),
            (ErrorCode::NotConfigured, "not_configured"),
            (ErrorCode::AuthenticationFailed, "authentication_failed"),
            (ErrorCode::Unavailable, "unavailable"),
            (ErrorCode::InvalidResponse, "invalid_response"),
            (ErrorCode::SecureStorage, "secure_storage"),
            (ErrorCode::NotConnected, "not_connected"),
            (ErrorCode::NotFound, "not_found"),
        ] {
            assert_eq!(
                serde_json::to_value(CommandError::new(code, "Safe message")).unwrap(),
                serde_json::json!({"code": wire_code, "message": "Safe message"})
            );
        }
    }
}
