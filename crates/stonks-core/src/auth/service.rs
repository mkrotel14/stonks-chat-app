use super::{
    credentials::{delete_token_if_matches, SessionCredential},
    models::{
        EmailActionRequest, LoginRequest, LoginResult, OnboardingRequest, PasswordResetRequest,
        SessionState, SignupRequest,
    },
    protocol::{
        auth_response_value, authenticated_session, email_action_body, login_request_body,
        login_result, onboarding_body, onboarding_session, operation_response_value,
        parse_login_response, password_reset_body, signup_body, unavailable, verification_code,
        verification_endpoint, AuthOperation, LoginResponse,
    },
};
use crate::{
    error::{CommandError, ErrorCode},
    http::{
        AuthenticatedContext, ContextError, DeploymentContext, HttpPolicy, HttpPolicyError,
        RequestClass, MAX_JSON_RESPONSE_BYTES,
    },
};

fn context_error(error: ContextError) -> CommandError {
    match error {
        ContextError::NotConfigured => {
            CommandError::new(ErrorCode::NotConfigured, "Choose a Stoat server first.")
        }
        ContextError::ConfigurationUnavailable => CommandError::new(
            ErrorCode::Unavailable,
            "Could not load the server configuration.",
        ),
        ContextError::SecureStorage => CommandError::new(
            ErrorCode::SecureStorage,
            "Could not access secure session storage.",
        ),
        ContextError::AuthenticationRequired => CommandError::new(
            ErrorCode::AuthenticationFailed,
            "Your session expired. Log in again.",
        ),
        ContextError::Cancelled => unavailable(),
    }
}

fn http_error(error: HttpPolicyError) -> CommandError {
    match error {
        HttpPolicyError::InvalidResponse | HttpPolicyError::BodyTooLarge => CommandError::new(
            ErrorCode::InvalidResponse,
            "The Stoat server returned an invalid response.",
        ),
        HttpPolicyError::Unavailable | HttpPolicyError::Timeout | HttpPolicyError::Cancelled => {
            unavailable()
        }
    }
}

fn deployment(policy: &HttpPolicy) -> Result<DeploymentContext, CommandError> {
    policy.require_deployment().map_err(context_error)
}

fn authenticated(policy: &HttpPolicy) -> Result<AuthenticatedContext, CommandError> {
    policy.authenticated_context().map_err(context_error)
}

async fn is_onboarding(
    policy: &HttpPolicy,
    context: &AuthenticatedContext,
) -> Result<bool, CommandError> {
    let request = policy
        .client()
        .get(format!("{}/onboard/hello", context.server.endpoints.api))
        .header("x-session-token", &context.token);
    let response = policy
        .execute_authenticated(
            context,
            request,
            RequestClass::Standard,
            MAX_JSON_RESPONSE_BYTES,
        )
        .await
        .map_err(http_error)?;
    let value = auth_response_value(response)?;
    Ok(value
        .get("onboarding")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false))
}

async fn fetch_user(
    policy: &HttpPolicy,
    context: &AuthenticatedContext,
) -> Result<SessionState, CommandError> {
    let request = policy
        .client()
        .get(format!("{}/users/@me", context.server.endpoints.api))
        .header("x-session-token", &context.token);
    let response = policy
        .execute_authenticated(
            context,
            request,
            RequestClass::Standard,
            MAX_JSON_RESPONSE_BYTES,
        )
        .await
        .map_err(http_error)?;
    authenticated_session(&auth_response_value(response)?)
}

async fn resolve_session(
    policy: &HttpPolicy,
    context: &AuthenticatedContext,
) -> Result<SessionState, CommandError> {
    if is_onboarding(policy, context).await? {
        Ok(SessionState::Onboarding)
    } else {
        fetch_user(policy, context).await
    }
}

pub async fn login(policy: &HttpPolicy, request: LoginRequest) -> Result<LoginResult, CommandError> {
    let body = login_request_body(&request)?;
    let _login_guard = policy.login_guard().await;
    let context = deployment(policy)?;
    let request = policy
        .client()
        .post(format!(
            "{}/auth/session/login",
            context.server.endpoints.api
        ))
        .json(&body);
    let response = policy
        .execute_deployment(
            &context,
            request,
            RequestClass::Standard,
            MAX_JSON_RESPONSE_BYTES,
        )
        .await
        .map_err(http_error)?;
    let response = parse_login_response(&auth_response_value(response)?)?;

    let session = match &response {
        LoginResponse::Success { id, token } => {
            let credential = SessionCredential::open(&context.server)?;
            credential.store(token, id)?;
            if !policy.activate_session(&context) {
                let _ = credential.delete();
                return Err(unavailable());
            }
            let authenticated = authenticated(policy)?;
            Some(resolve_session(policy, &authenticated).await?)
        }
        LoginResponse::Mfa { .. } | LoginResponse::Disabled => None,
    };
    login_result(response, session)
}

pub async fn create_account(
    policy: &HttpPolicy,
    request: SignupRequest,
) -> Result<(), CommandError> {
    let body = signup_body(&request)?;
    let context = deployment(policy)?;
    let request = policy
        .client()
        .post(format!(
            "{}/auth/account/create",
            context.server.endpoints.api
        ))
        .json(&body);
    let response = policy
        .execute_deployment(
            &context,
            request,
            RequestClass::Standard,
            MAX_JSON_RESPONSE_BYTES,
        )
        .await
        .map_err(http_error)?;
    operation_response_value(response, AuthOperation::CreateAccount)?;
    Ok(())
}

async fn email_action(
    policy: &HttpPolicy,
    request: EmailActionRequest,
    endpoint: &str,
    operation: AuthOperation,
) -> Result<(), CommandError> {
    let body = email_action_body(&request)?;
    let context = deployment(policy)?;
    let request = policy
        .client()
        .post(format!("{}{endpoint}", context.server.endpoints.api))
        .json(&body);
    let response = policy
        .execute_deployment(
            &context,
            request,
            RequestClass::Standard,
            MAX_JSON_RESPONSE_BYTES,
        )
        .await
        .map_err(http_error)?;
    operation_response_value(response, operation)?;
    Ok(())
}

pub async fn resend_verification(
    policy: &HttpPolicy,
    request: EmailActionRequest,
) -> Result<(), CommandError> {
    email_action(
        policy,
        request,
        "/auth/account/reverify",
        AuthOperation::ResendVerification,
    )
    .await
}

pub async fn send_password_reset(
    policy: &HttpPolicy,
    request: EmailActionRequest,
) -> Result<(), CommandError> {
    email_action(
        policy,
        request,
        "/auth/account/reset_password",
        AuthOperation::SendPasswordReset,
    )
    .await
}

pub async fn verify_email(policy: &HttpPolicy, code: String) -> Result<(), CommandError> {
    let code = verification_code(&code)?;
    let context = deployment(policy)?;
    let endpoint = verification_endpoint(&context.server.endpoints.api, &code)?;
    let request = policy.client().post(endpoint);
    let response = policy
        .execute_deployment(
            &context,
            request,
            RequestClass::Standard,
            MAX_JSON_RESPONSE_BYTES,
        )
        .await
        .map_err(http_error)?;
    operation_response_value(response, AuthOperation::VerifyEmail)?;
    Ok(())
}

pub async fn confirm_password_reset(
    policy: &HttpPolicy,
    request: PasswordResetRequest,
) -> Result<(), CommandError> {
    let body = password_reset_body(&request)?;
    let context = deployment(policy)?;
    let request = policy
        .client()
        .patch(format!(
            "{}/auth/account/reset_password",
            context.server.endpoints.api
        ))
        .json(&body);
    let response = policy
        .execute_deployment(
            &context,
            request,
            RequestClass::Standard,
            MAX_JSON_RESPONSE_BYTES,
        )
        .await
        .map_err(http_error)?;
    operation_response_value(response, AuthOperation::ConfirmPasswordReset)?;
    Ok(())
}

pub async fn complete_onboarding(
    policy: &HttpPolicy,
    request: OnboardingRequest,
) -> Result<SessionState, CommandError> {
    let body = onboarding_body(&request)?;
    let context = authenticated(policy)?;
    let request = policy
        .client()
        .post(format!("{}/onboard/complete", context.server.endpoints.api))
        .header("x-session-token", &context.token)
        .json(&body);
    let response = policy
        .execute_authenticated(
            &context,
            request,
            RequestClass::Standard,
            MAX_JSON_RESPONSE_BYTES,
        )
        .await
        .map_err(http_error)?;
    let value = operation_response_value(response, AuthOperation::CompleteOnboarding)?;
    onboarding_session(&value)
}

pub async fn load_session(policy: &HttpPolicy) -> Result<Option<SessionState>, CommandError> {
    let Some(deployment) = policy.deployment_context().map_err(context_error)? else {
        return Ok(None);
    };
    if SessionCredential::open(&deployment.server)?
        .load()?
        .is_none()
    {
        return Ok(None);
    }
    let context = authenticated(policy)?;
    Ok(Some(resolve_session(policy, &context).await?))
}

pub async fn logout(policy: &HttpPolicy) -> Result<(), CommandError> {
    let Some(context) = policy.deployment_context().map_err(context_error)? else {
        policy.logout();
        return Ok(());
    };
    let credential = SessionCredential::open(&context.server)?;
    let token = credential.load()?;

    policy.logout();
    if let Some(token) = token {
        delete_token_if_matches(&context.server, &token).map_err(|_| {
            CommandError::new(
                ErrorCode::SecureStorage,
                "Could not access secure session storage.",
            )
        })?;
        let request = policy
            .client()
            .post(format!(
                "{}/auth/session/logout",
                context.server.endpoints.api
            ))
            .header("x-session-token", token)
            .timeout(std::time::Duration::from_secs(3))
            .send();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(3), request).await;
    }
    Ok(())
}
