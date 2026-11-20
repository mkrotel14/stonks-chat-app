use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    pub email: Option<String>,
    pub password: Option<String>,
    pub mfa_ticket: Option<String>,
    pub mfa_code: Option<String>,
    pub mfa_method: Option<MfaMethod>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum MfaMethod {
    Password,
    Recovery,
    Totp,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignupRequest {
    pub email: String,
    pub password: String,
    pub invite: Option<String>,
    pub captcha: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct EmailActionRequest {
    pub email: String,
    pub captcha: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasswordResetRequest {
    pub token: String,
    pub password: String,
    pub remove_sessions: bool,
}

#[derive(Debug, Deserialize)]
pub struct OnboardingRequest {
    pub username: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum LoginResult {
    Authenticated {
        #[serde(rename = "userId")]
        user_id: String,
        username: String,
    },
    Onboarding,
    Mfa {
        ticket: String,
        #[serde(rename = "allowedMethods")]
        allowed_methods: Vec<String>,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum SessionState {
    Authenticated {
        #[serde(rename = "userId")]
        user_id: String,
        username: String,
    },
    Onboarding,
}
