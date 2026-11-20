use reqwest::StatusCode;
use serde_json::{json, Value};
use url::Url;

use super::models::{
    EmailActionRequest, LoginRequest, LoginResult, MfaMethod, OnboardingRequest,
    PasswordResetRequest, SessionState, SignupRequest,
};
use crate::http::BoundedResponse;
use crate::error::{CommandError, ErrorCode};

#[derive(Clone, Copy)]
pub enum AuthOperation {
    CreateAccount,
    ResendVerification,
    SendPasswordReset,
    VerifyEmail,
    ConfirmPasswordReset,
    CompleteOnboarding,
}

pub enum LoginResponse {
    Mfa {
        ticket: String,
        allowed_methods: Vec<String>,
    },
    Success {
        id: String,
        token: String,
    },
    Disabled,
}

fn invalid_input(message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::InvalidInput, message)
}

fn invalid_response() -> CommandError {
    CommandError::new(
        ErrorCode::InvalidResponse,
        "The Stoat server returned an invalid response.",
    )
}

fn required_trimmed(value: &str, message: &'static str) -> Result<String, CommandError> {
    let value = value.trim();
    if value.is_empty() {
        Err(invalid_input(message))
    } else {
        Ok(value.to_owned())
    }
}

fn required_password(value: &str, message: &'static str) -> Result<String, CommandError> {
    if value.trim().is_empty() {
        Err(invalid_input(message))
    } else {
        Ok(value.to_owned())
    }
}

pub fn login_request_body(request: &LoginRequest) -> Result<Value, CommandError> {
    if let Some(ticket) = request.mfa_ticket.as_deref() {
        let code = request
            .mfa_code
            .as_deref()
            .filter(|code| !code.trim().is_empty())
            .ok_or(CommandError::new(
                ErrorCode::InvalidInput,
                "Enter your authentication code.",
            ))?;
        let response = match request.mfa_method.as_ref().unwrap_or(&MfaMethod::Totp) {
            MfaMethod::Password => json!({ "password": code }),
            MfaMethod::Recovery => json!({ "recovery_code": code }),
            MfaMethod::Totp => json!({ "totp_code": code }),
        };
        return Ok(
            json!({ "mfa_ticket": ticket, "mfa_response": response, "friendly_name": "Stonks Chat Desktop" }),
        );
    }
    let email = request
        .email
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or(CommandError::new(
            ErrorCode::InvalidInput,
            "Enter your email address.",
        ))?;
    let password = request
        .password
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or(CommandError::new(
            ErrorCode::InvalidInput,
            "Enter your password.",
        ))?;
    Ok(
        json!({ "email": email.trim(), "password": password, "friendly_name": "Stonks Chat Desktop" }),
    )
}

pub fn signup_body(request: &SignupRequest) -> Result<Value, CommandError> {
    let mut body = json!({
        "email": required_trimmed(&request.email, "Enter your email address.")?,
        "password": required_password(&request.password, "Enter a password.")?,
    });
    if let Some(invite) = request
        .invite
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        body["invite"] = Value::String(invite.to_owned());
    }
    if let Some(captcha) = request.captcha.as_deref().filter(|v| !v.trim().is_empty()) {
        body["captcha"] = Value::String(captcha.to_owned());
    }
    Ok(body)
}

pub fn email_action_body(request: &EmailActionRequest) -> Result<Value, CommandError> {
    let mut body =
        json!({ "email": required_trimmed(&request.email, "Enter your email address.")? });
    if let Some(captcha) = request.captcha.as_deref().filter(|v| !v.trim().is_empty()) {
        body["captcha"] = Value::String(captcha.to_owned());
    }
    Ok(body)
}

pub fn password_reset_body(request: &PasswordResetRequest) -> Result<Value, CommandError> {
    Ok(json!({
        "token": required_trimmed(&request.token, "Enter the reset token.")?,
        "password": required_password(&request.password, "Enter a new password.")?,
        "remove_sessions": request.remove_sessions,
    }))
}

pub fn onboarding_body(request: &OnboardingRequest) -> Result<Value, CommandError> {
    Ok(json!({
        "username": required_trimmed(&request.username, "Enter a username.")?,
    }))
}

pub fn verification_code(value: &str) -> Result<String, CommandError> {
    let code = required_trimmed(value, "Enter the verification code.")?;
    if code == "." || code == ".." {
        Err(invalid_input(
            "The verification code could not be accepted.",
        ))
    } else {
        Ok(code)
    }
}

pub fn verification_endpoint(api: &str, code: &str) -> Result<Url, CommandError> {
    let mut endpoint = Url::parse(api).map_err(|_| {
        CommandError::new(
            ErrorCode::Unavailable,
            "Could not use the server configuration.",
        )
    })?;
    endpoint
        .path_segments_mut()
        .map_err(|_| {
            CommandError::new(
                ErrorCode::Unavailable,
                "Could not use the server configuration.",
            )
        })?
        .extend(["auth", "account", "verify", code]);
    Ok(endpoint)
}

pub fn parse_login_response(value: &Value) -> Result<LoginResponse, CommandError> {
    match value.get("result").and_then(Value::as_str) {
        Some("MFA") => Ok(LoginResponse::Mfa {
            ticket: value
                .get("ticket")
                .and_then(Value::as_str)
                .ok_or_else(invalid_response)?
                .into(),
            allowed_methods: value
                .get("allowed_methods")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect(),
        }),
        Some("Success") => Ok(LoginResponse::Success {
            id: value
                .get("_id")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
                .ok_or_else(invalid_response)?
                .to_owned(),
            token: value
                .get("token")
                .and_then(Value::as_str)
                .filter(|token| !token.is_empty())
                .ok_or_else(invalid_response)?
                .to_owned(),
        }),
        Some("Disabled") => Ok(LoginResponse::Disabled),
        _ => Err(invalid_response()),
    }
}

pub fn login_result(
    response: LoginResponse,
    session: Option<SessionState>,
) -> Result<LoginResult, CommandError> {
    match response {
        LoginResponse::Mfa {
            ticket,
            allowed_methods,
        } => Ok(LoginResult::Mfa {
            ticket,
            allowed_methods,
        }),
        LoginResponse::Success { .. } => match session.ok_or_else(invalid_response)? {
            SessionState::Authenticated { user_id, username } => {
                Ok(LoginResult::Authenticated { user_id, username })
            }
            SessionState::Onboarding => Ok(LoginResult::Onboarding),
        },
        LoginResponse::Disabled => Err(CommandError::new(
            ErrorCode::AuthenticationFailed,
            "This account is disabled.",
        )),
    }
}

pub fn onboarding_session(value: &Value) -> Result<SessionState, CommandError> {
    authenticated_session(value)
}

pub fn authenticated_session(value: &Value) -> Result<SessionState, CommandError> {
    let user_id = value
        .get("_id")
        .and_then(Value::as_str)
        .ok_or_else(invalid_response)?
        .to_owned();
    let username = value
        .get("username")
        .and_then(Value::as_str)
        .ok_or_else(invalid_response)?
        .to_owned();
    Ok(SessionState::Authenticated { user_id, username })
}

pub fn operation_response_value(
    response: BoundedResponse,
    operation: AuthOperation,
) -> Result<Value, CommandError> {
    response.consume(
        |status, _, bytes| operation_response_bytes(status, bytes, operation),
        unavailable,
    )
}

fn operation_response_bytes(
    status: StatusCode,
    bytes: &[u8],
    operation: AuthOperation,
) -> Result<Value, CommandError> {
    let value = serde_json::from_slice(bytes).unwrap_or(Value::Null);
    if status.is_success() {
        Ok(value)
    } else {
        Err(operation_error(operation, status, &value))
    }
}

fn operation_error(operation: AuthOperation, status: StatusCode, value: &Value) -> CommandError {
    if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        return unavailable();
    }

    if status.is_client_error() {
        let error_type = value.get("type").and_then(Value::as_str);
        let known = match (operation, error_type) {
            (_, Some("CaptchaFailed")) => Some(CommandError::new(
                ErrorCode::AuthenticationFailed,
                "Captcha verification failed. Please try again.",
            )),
            (AuthOperation::CreateAccount, Some("IncorrectData")) => {
                Some(invalid_input("The account details were not accepted."))
            }
            (AuthOperation::CreateAccount, Some("MissingInvite")) => {
                Some(invalid_input("An invite is required to create an account."))
            }
            (AuthOperation::CreateAccount, Some("InvalidInvite")) => {
                Some(invalid_input("The invite is invalid or expired."))
            }
            (
                AuthOperation::CreateAccount | AuthOperation::ConfirmPasswordReset,
                Some("CompromisedPassword"),
            ) => Some(invalid_input(
                "Choose a password that has not been compromised.",
            )),
            (
                AuthOperation::CreateAccount | AuthOperation::ConfirmPasswordReset,
                Some("ShortPassword"),
            ) => Some(invalid_input("Choose a longer password.")),
            (AuthOperation::VerifyEmail, Some("InvalidToken")) => Some(CommandError::new(
                ErrorCode::AuthenticationFailed,
                "The verification code is invalid or expired.",
            )),
            (AuthOperation::ConfirmPasswordReset, Some("InvalidToken")) => Some(CommandError::new(
                ErrorCode::AuthenticationFailed,
                "The password reset token is invalid or expired.",
            )),
            (AuthOperation::CompleteOnboarding, Some("InvalidUsername" | "FailedValidation")) => {
                Some(invalid_input("The username is invalid."))
            }
            (AuthOperation::CompleteOnboarding, Some("UsernameTaken")) => {
                Some(invalid_input("That username is already taken."))
            }
            (AuthOperation::CompleteOnboarding, Some("AlreadyOnboarded")) => {
                Some(CommandError::new(
                    ErrorCode::AuthenticationFailed,
                    "Account setup has already been completed.",
                ))
            }
            (AuthOperation::CompleteOnboarding, Some("InvalidSession" | "NotAuthenticated")) => {
                Some(CommandError::new(
                    ErrorCode::AuthenticationFailed,
                    "Your session expired. Log in again.",
                ))
            }
            _ => None,
        };
        if let Some(error) = known {
            return error;
        }

        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return CommandError::new(
                ErrorCode::AuthenticationFailed,
                match operation {
                    AuthOperation::VerifyEmail => "The verification code could not be accepted.",
                    AuthOperation::ConfirmPasswordReset => {
                        "The password reset could not be authenticated."
                    }
                    AuthOperation::CompleteOnboarding => "Your session expired. Log in again.",
                    AuthOperation::CreateAccount => "The account request could not be verified.",
                    AuthOperation::ResendVerification => {
                        "The verification request could not be authenticated."
                    }
                    AuthOperation::SendPasswordReset => {
                        "The password reset request could not be authenticated."
                    }
                },
            );
        }

        return invalid_input(match operation {
            AuthOperation::CreateAccount => "The account details were not accepted.",
            AuthOperation::ResendVerification => "The verification request was not accepted.",
            AuthOperation::SendPasswordReset => "The password reset request was not accepted.",
            AuthOperation::VerifyEmail => "The verification code could not be accepted.",
            AuthOperation::ConfirmPasswordReset => "The new password was not accepted.",
            AuthOperation::CompleteOnboarding => "The username could not be accepted.",
        });
    }

    unavailable()
}

pub fn auth_response_value(response: BoundedResponse) -> Result<Value, CommandError> {
    response.consume(
        |status, _, bytes| {
            let value = if bytes.is_empty() {
                Value::Null
            } else {
                match serde_json::from_slice(bytes) {
                    Ok(value) => value,
                    Err(_) if !status.is_success() => Value::Null,
                    Err(_) => return Err(invalid_response()),
                }
            };
            if status.is_success() {
                Ok(value)
            } else {
                Err(auth_status_error(status, &value))
            }
        },
        unavailable,
    )
}

fn auth_status_error(status: StatusCode, value: &Value) -> CommandError {
    let is_auth_rejection = status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN;
    let is_captcha_rejection = is_auth_rejection
        && value
            .get("error")
            .and_then(Value::as_str)
            .or_else(|| value.get("message").and_then(Value::as_str))
            .or_else(|| value.get("type").and_then(Value::as_str))
            .is_some_and(|message| message.to_ascii_lowercase().contains("captcha"));

    if is_captcha_rejection {
        CommandError::new(
            ErrorCode::AuthenticationFailed,
            "Captcha verification failed. Please try again.",
        )
    } else if is_auth_rejection {
        CommandError::new(
            ErrorCode::AuthenticationFailed,
            "The server could not verify those details.",
        )
    } else {
        unavailable()
    }
}

pub fn unavailable() -> CommandError {
    CommandError::new(
        ErrorCode::Unavailable,
        "The Stoat server is unavailable. Try again.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error_value(error: CommandError) -> Value {
        serde_json::to_value(error).unwrap()
    }

    #[test]
    fn creates_email_login_body() {
        let body = login_request_body(&LoginRequest {
            email: Some(" user@example.com ".into()),
            password: Some("secret".into()),
            mfa_ticket: None,
            mfa_code: None,
            mfa_method: None,
        })
        .unwrap();
        assert_eq!(body["email"], "user@example.com");
        assert_eq!(body["friendly_name"], "Stonks Chat Desktop");
    }

    #[test]
    fn successful_login_preserves_the_session_id_for_secure_storage() {
        let response = parse_login_response(&serde_json::json!({
            "result": "Success",
            "_id": "session-id",
            "token": "secret-token",
            "last_seen": "2026-09-27T12:00:00Z"
        }))
        .unwrap();
        match response {
            LoginResponse::Success { id, token } => {
                assert_eq!(id, "session-id");
                assert_eq!(token, "secret-token");
            }
            _ => panic!("expected a successful login"),
        }

        assert!(parse_login_response(&serde_json::json!({
            "result": "Success",
            "token": "secret-token"
        }))
        .is_err());
    }

    #[test]
    fn creates_totp_login_body() {
        let body = login_request_body(&LoginRequest {
            email: None,
            password: None,
            mfa_ticket: Some("ticket".into()),
            mfa_code: Some("123456".into()),
            mfa_method: Some(MfaMethod::Totp),
        })
        .unwrap();
        assert_eq!(body["mfa_response"]["totp_code"], "123456");
    }

    #[test]
    fn signup_validation_trims_email_and_invite_but_preserves_password() {
        let body = signup_body(&SignupRequest {
            email: " user@example.com ".into(),
            password: "  secret phrase  ".into(),
            invite: Some(" invite-code ".into()),
            captcha: Some(" captcha-token ".into()),
        })
        .unwrap();

        assert_eq!(body["email"], "user@example.com");
        assert_eq!(body["password"], "  secret phrase  ");
        assert_eq!(body["invite"], "invite-code");
        assert_eq!(body["captcha"], " captcha-token ");
    }

    #[test]
    fn password_validation_rejects_whitespace_only_without_normalizing_valid_passwords() {
        let error = signup_body(&SignupRequest {
            email: "user@example.com".into(),
            password: " \t\n ".into(),
            invite: None,
            captcha: None,
        })
        .unwrap_err();
        assert_eq!(
            error_value(error),
            serde_json::json!({"code": "invalid_input", "message": "Enter a password."})
        );

        let body = password_reset_body(&PasswordResetRequest {
            token: " reset-token ".into(),
            password: " new password ".into(),
            remove_sessions: true,
        })
        .unwrap();
        assert_eq!(body["token"], "reset-token");
        assert_eq!(body["password"], " new password ");
    }

    #[test]
    fn login_validation_returns_a_stable_typed_error() {
        let error = login_request_body(&LoginRequest {
            email: Some("   ".into()),
            password: Some("secret".into()),
            mfa_ticket: None,
            mfa_code: None,
            mfa_method: None,
        })
        .unwrap_err();

        assert_eq!(
            error_value(error),
            serde_json::json!({
                "code": "invalid_input",
                "message": "Enter your email address."
            })
        );
    }

    #[test]
    fn auth_http_statuses_map_without_backend_details() {
        for status in [StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN] {
            assert_eq!(
                error_value(auth_status_error(status, &Value::Null)),
                serde_json::json!({
                    "code": "authentication_failed",
                    "message": "The server could not verify those details."
                })
            );
        }
        assert_eq!(
            error_value(auth_status_error(
                StatusCode::UNAUTHORIZED,
                &serde_json::json!({"error": "Invalid Captcha response: private detail"}),
            )),
            serde_json::json!({
                "code": "authentication_failed",
                "message": "Captcha verification failed. Please try again."
            })
        );
        assert_eq!(
            error_value(auth_status_error(
                StatusCode::BAD_GATEWAY,
                &serde_json::json!({"message": "/private/backend/path"}),
            )),
            serde_json::json!({
                "code": "unavailable",
                "message": "The Stoat server is unavailable. Try again."
            })
        );
    }

    #[test]
    fn known_backend_types_map_to_static_operation_errors() {
        let cases = [
            (
                AuthOperation::CreateAccount,
                "CaptchaFailed",
                "authentication_failed",
                "Captcha verification failed. Please try again.",
            ),
            (
                AuthOperation::CreateAccount,
                "IncorrectData",
                "invalid_input",
                "The account details were not accepted.",
            ),
            (
                AuthOperation::CreateAccount,
                "MissingInvite",
                "invalid_input",
                "An invite is required to create an account.",
            ),
            (
                AuthOperation::CreateAccount,
                "InvalidInvite",
                "invalid_input",
                "The invite is invalid or expired.",
            ),
            (
                AuthOperation::CreateAccount,
                "CompromisedPassword",
                "invalid_input",
                "Choose a password that has not been compromised.",
            ),
            (
                AuthOperation::ConfirmPasswordReset,
                "ShortPassword",
                "invalid_input",
                "Choose a longer password.",
            ),
            (
                AuthOperation::VerifyEmail,
                "InvalidToken",
                "authentication_failed",
                "The verification code is invalid or expired.",
            ),
            (
                AuthOperation::ConfirmPasswordReset,
                "InvalidToken",
                "authentication_failed",
                "The password reset token is invalid or expired.",
            ),
            (
                AuthOperation::CompleteOnboarding,
                "InvalidUsername",
                "invalid_input",
                "The username is invalid.",
            ),
            (
                AuthOperation::CompleteOnboarding,
                "FailedValidation",
                "invalid_input",
                "The username is invalid.",
            ),
            (
                AuthOperation::CompleteOnboarding,
                "UsernameTaken",
                "invalid_input",
                "That username is already taken.",
            ),
            (
                AuthOperation::CompleteOnboarding,
                "AlreadyOnboarded",
                "authentication_failed",
                "Account setup has already been completed.",
            ),
            (
                AuthOperation::CompleteOnboarding,
                "InvalidSession",
                "authentication_failed",
                "Your session expired. Log in again.",
            ),
            (
                AuthOperation::CompleteOnboarding,
                "NotAuthenticated",
                "authentication_failed",
                "Your session expired. Log in again.",
            ),
        ];

        for (operation, backend_type, code, message) in cases {
            let error = operation_error(
                operation,
                StatusCode::BAD_REQUEST,
                &serde_json::json!({
                    "type": backend_type,
                    "message": "private backend detail",
                    "token": "secret-token"
                }),
            );
            assert_eq!(
                error_value(error),
                serde_json::json!({"code": code, "message": message})
            );
        }
    }

    #[test]
    fn unknown_and_inexact_backend_errors_use_safe_operation_fallbacks() {
        let unknown = error_value(operation_error(
            AuthOperation::SendPasswordReset,
            StatusCode::BAD_REQUEST,
            &serde_json::json!({
                "type": "InternalPrivateType",
                "message": "request to https://private.example/reset failed",
                "token": "secret-token"
            }),
        ));
        assert_eq!(
            unknown,
            serde_json::json!({
                "code": "invalid_input",
                "message": "The password reset request was not accepted."
            })
        );

        let inexact = error_value(operation_error(
            AuthOperation::CreateAccount,
            StatusCode::UNAUTHORIZED,
            &serde_json::json!({"type": "captchafailed"}),
        ));
        assert_eq!(
            inexact,
            serde_json::json!({
                "code": "authentication_failed",
                "message": "The account request could not be verified."
            })
        );
    }

    #[test]
    fn rate_limits_server_failures_and_other_non_client_statuses_are_unavailable() {
        for status in [
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
            StatusCode::FOUND,
        ] {
            assert_eq!(
                error_value(operation_error(
                    AuthOperation::CreateAccount,
                    status,
                    &serde_json::json!({"message": "private detail"}),
                )),
                serde_json::json!({
                    "code": "unavailable",
                    "message": "The Stoat server is unavailable. Try again."
                })
            );
        }
    }

    #[test]
    fn empty_success_is_preserved_and_malformed_onboarding_success_is_rejected() {
        assert_eq!(
            operation_response_bytes(
                StatusCode::NO_CONTENT,
                b"",
                AuthOperation::ResendVerification,
            )
            .unwrap(),
            Value::Null
        );
        assert_eq!(
            error_value(
                onboarding_session(
                    &operation_response_bytes(
                        StatusCode::OK,
                        b"not json",
                        AuthOperation::CompleteOnboarding,
                    )
                    .unwrap(),
                )
                .unwrap_err()
            ),
            serde_json::json!({
                "code": "invalid_response",
                "message": "The Stoat server returned an invalid response."
            })
        );
    }

    #[test]
    fn verification_code_is_encoded_as_one_path_segment() {
        let endpoint =
            verification_endpoint("https://api.example.test/v1", "code/with ?#% parts").unwrap();
        assert_eq!(
            endpoint.as_str(),
            "https://api.example.test/v1/auth/account/verify/code%2Fwith%20%3F%23%25%20parts"
        );
        assert_eq!(
            error_value(verification_code("..").unwrap_err()),
            serde_json::json!({
                "code": "invalid_input",
                "message": "The verification code could not be accepted."
            })
        );
    }
}
