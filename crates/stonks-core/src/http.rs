use futures_util::future::{AbortHandle, Abortable};
use futures_util::StreamExt;
use reqwest::{header::HeaderMap, Client, RequestBuilder, StatusCode};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::server::models::ServerSelection;

/// Where request contexts get their server selection and session token.
/// Implemented by `crate::session::SessionStore` in production; the default
/// is unconfigured (used by tests and before the host app picks a config
/// directory).
pub trait SessionAccess: Send + Sync {
    fn server_selection(&self) -> Result<Option<ServerSelection>, ContextError>;
    fn token(&self, server: &ServerSelection) -> Result<Option<String>, ContextError>;
}

struct MissingSessionAccess;

impl SessionAccess for MissingSessionAccess {
    fn server_selection(&self) -> Result<Option<ServerSelection>, ContextError> {
        Ok(None)
    }

    fn token(&self, _server: &ServerSelection) -> Result<Option<String>, ContextError> {
        Ok(None)
    }
}

pub const MAX_JSON_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_ERROR_RESPONSE_BYTES: u64 = 64 * 1024;
pub const MAX_JSON_REQUEST_BYTES: u64 = 2 * 1024 * 1024;
pub const MAX_TRANSFER_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;
const STANDARD_TIMEOUT: Duration = Duration::from_secs(30);
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestClass {
    Standard,
    Transfer,
}

impl RequestClass {
    fn timeout(self) -> Duration {
        match self {
            Self::Standard => STANDARD_TIMEOUT,
            Self::Transfer => TRANSFER_TIMEOUT,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RequestScope {
    Deployment,
    Authenticated,
}

#[derive(Clone, Debug)]
pub struct DeploymentContext {
    pub server: ServerSelection,
    epoch: u64,
}

#[derive(Clone, Debug)]
pub struct AuthenticatedContext {
    pub server: ServerSelection,
    pub token: String,
    epoch: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextError {
    NotConfigured,
    ConfigurationUnavailable,
    SecureStorage,
    AuthenticationRequired,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HttpPolicyError {
    Unavailable,
    Timeout,
    InvalidResponse,
    BodyTooLarge,
    Cancelled,
}

pub struct BoundedResponse {
    status: StatusCode,
    headers: HeaderMap,
    bytes: Vec<u8>,
    guard: RequestGuard,
}

impl BoundedResponse {
    pub fn consume<T, E>(
        mut self,
        parse: impl FnOnce(StatusCode, &HeaderMap, &[u8]) -> Result<T, E>,
        cancelled: impl FnOnce() -> E,
    ) -> Result<T, E> {
        let result = parse(self.status, &self.headers, &self.bytes);
        if !self.guard.finish() {
            return Err(cancelled());
        }
        result
    }
}

#[derive(Clone, Copy)]
struct ExecutionContext {
    scope: RequestScope,
    epoch: u64,
}

impl From<&DeploymentContext> for ExecutionContext {
    fn from(context: &DeploymentContext) -> Self {
        Self {
            scope: RequestScope::Deployment,
            epoch: context.epoch,
        }
    }
}

impl From<&AuthenticatedContext> for ExecutionContext {
    fn from(context: &AuthenticatedContext) -> Self {
        Self {
            scope: RequestScope::Authenticated,
            epoch: context.epoch,
        }
    }
}

struct ActiveRequest {
    epoch: u64,
    scope: RequestScope,
    abort: AbortHandle,
}

struct PolicyState {
    epoch: u64,
    next_request: u64,
    authenticated_enabled: bool,
    shutting_down: bool,
    active: HashMap<u64, ActiveRequest>,
}

impl Default for PolicyState {
    fn default() -> Self {
        Self {
            epoch: 0,
            next_request: 0,
            authenticated_enabled: true,
            shutting_down: false,
            active: HashMap::new(),
        }
    }
}

pub struct HttpPolicy {
    client: Client,
    state: Arc<Mutex<PolicyState>>,
    login: tokio::sync::Mutex<()>,
    sessions: Arc<dyn SessionAccess>,
}

impl Default for HttpPolicy {
    fn default() -> Self {
        Self::new(Arc::new(MissingSessionAccess))
            .expect("could not create the shared native HTTP client")
    }
}

impl HttpPolicy {
    pub fn new(sessions: Arc<dyn SessionAccess>) -> Result<Self, reqwest::Error> {
        let client = Client::builder()
            .user_agent(format!("Stonks/{}", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(Duration::from_secs(20))
            .pool_idle_timeout(Duration::from_secs(90))
            .build()?;
        Ok(Self {
            client,
            state: Arc::new(Mutex::new(PolicyState::default())),
            login: tokio::sync::Mutex::new(()),
            sessions,
        })
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub async fn login_guard(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.login.lock().await
    }

    pub fn deployment_context(&self) -> Result<Option<DeploymentContext>, ContextError> {
        let epoch = self.current_epoch(RequestScope::Deployment)?;
        let server = self.sessions.server_selection()?;
        Ok(server.map(|server| DeploymentContext { server, epoch }))
    }

    pub fn require_deployment(&self) -> Result<DeploymentContext, ContextError> {
        self.deployment_context()?
            .ok_or(ContextError::NotConfigured)
    }

    pub fn authenticated_context(&self) -> Result<AuthenticatedContext, ContextError> {
        let epoch = self.current_epoch(RequestScope::Authenticated)?;
        let server = self
            .sessions
            .server_selection()?
            .ok_or(ContextError::NotConfigured)?;
        let token = self
            .sessions
            .token(&server)?
            .ok_or(ContextError::AuthenticationRequired)?;
        if self.current_epoch(RequestScope::Authenticated)? != epoch {
            return Err(ContextError::Cancelled);
        }
        Ok(AuthenticatedContext {
            server,
            token,
            epoch,
        })
    }

    pub fn is_authenticated_context_current(&self, context: &AuthenticatedContext) -> bool {
        self.current_epoch(RequestScope::Authenticated) == Ok(context.epoch)
    }

    pub async fn execute_deployment(
        &self,
        context: &DeploymentContext,
        request: RequestBuilder,
        class: RequestClass,
        max_response_bytes: u64,
    ) -> Result<BoundedResponse, HttpPolicyError> {
        self.execute(context.into(), request, class, max_response_bytes)
            .await
    }

    pub async fn run_deployment<T, E, F>(
        &self,
        context: &DeploymentContext,
        operation: F,
        cancelled: impl Fn() -> E,
    ) -> Result<T, E>
    where
        F: std::future::Future<Output = Result<T, E>>,
    {
        let execution = ExecutionContext::from(context);
        let (request_id, registration) = self.begin(execution).map_err(|_| cancelled())?;
        let mut guard = RequestGuard {
            state: Arc::clone(&self.state),
            request_id: Some(request_id),
        };
        let result = Abortable::new(operation, registration)
            .await
            .map_err(|_| cancelled())?;
        if !guard.finish() {
            return Err(cancelled());
        }
        result
    }

    #[cfg(all(
        feature = "native-voice",
        any(target_os = "linux", target_os = "windows")
    ))]
    pub(crate) async fn authenticated_scope_ended(&self, context: &AuthenticatedContext) {
        let Ok((request_id, registration)) = self.begin(context.into()) else {
            return;
        };
        let _guard = RequestGuard {
            state: Arc::clone(&self.state),
            request_id: Some(request_id),
        };
        let _ = Abortable::new(std::future::pending::<()>(), registration).await;
    }

    pub async fn execute_authenticated(
        &self,
        context: &AuthenticatedContext,
        request: RequestBuilder,
        class: RequestClass,
        max_response_bytes: u64,
    ) -> Result<BoundedResponse, HttpPolicyError> {
        self.execute(context.into(), request, class, max_response_bytes)
            .await
    }

    async fn execute(
        &self,
        context: ExecutionContext,
        request: RequestBuilder,
        class: RequestClass,
        max_response_bytes: u64,
    ) -> Result<BoundedResponse, HttpPolicyError> {
        let request = request
            .timeout(class.timeout())
            .build()
            .map_err(|_| HttpPolicyError::Unavailable)?;
        if request
            .body()
            .and_then(reqwest::Body::as_bytes)
            .is_some_and(|body| body.len() as u64 > MAX_JSON_REQUEST_BYTES)
        {
            return Err(HttpPolicyError::BodyTooLarge);
        }

        let (request_id, registration) = self.begin(context)?;
        let guard = RequestGuard {
            state: Arc::clone(&self.state),
            request_id: Some(request_id),
        };
        let operation = async {
            let response = self.client.execute(request).await.map_err(|error| {
                if error.is_timeout() {
                    HttpPolicyError::Timeout
                } else {
                    HttpPolicyError::Unavailable
                }
            })?;
            let status = response.status();
            let headers = response.headers().clone();
            let body_limit = if status.is_success() {
                max_response_bytes
            } else {
                max_response_bytes.min(MAX_ERROR_RESPONSE_BYTES)
            };
            let bytes = read_bounded_body(response, body_limit, status.is_success()).await?;
            Ok((status, headers, bytes))
        };
        match Abortable::new(operation, registration).await {
            Ok(Ok((status, headers, bytes))) => Ok(BoundedResponse {
                status,
                headers,
                bytes,
                guard,
            }),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(HttpPolicyError::Cancelled),
        }
    }

    pub fn activate_session(&self, context: &DeploymentContext) -> bool {
        self.activate_epoch(context.epoch)
    }

    fn activate_epoch(&self, epoch: u64) -> bool {
        let aborts = {
            let mut state = self.lock_state();
            if state.shutting_down || state.epoch != epoch {
                return false;
            }
            state.epoch = state.epoch.wrapping_add(1);
            state.authenticated_enabled = true;
            state
                .active
                .drain()
                .map(|(_, request)| request.abort)
                .collect::<Vec<_>>()
        };
        for abort in aborts {
            abort.abort();
        }
        true
    }

    pub fn logout(&self) {
        self.invalidate(true, false);
    }

    pub fn logout_context(&self, context: &AuthenticatedContext) -> bool {
        self.logout_epoch(context.epoch)
    }

    fn logout_epoch(&self, epoch: u64) -> bool {
        let aborts = {
            let mut state = self.lock_state();
            if state.shutting_down || !state.authenticated_enabled || state.epoch != epoch {
                return false;
            }
            state.epoch = state.epoch.wrapping_add(1);
            state.authenticated_enabled = false;
            state
                .active
                .drain()
                .map(|(_, request)| request.abort)
                .collect::<Vec<_>>()
        };
        for abort in aborts {
            abort.abort();
        }
        true
    }

    pub fn deployment_replaced(&self) {
        self.invalidate(false, true);
    }

    pub fn shutdown(&self) {
        let aborts = {
            let mut state = self.lock_state();
            state.shutting_down = true;
            state.authenticated_enabled = false;
            state.epoch = state.epoch.wrapping_add(1);
            state
                .active
                .drain()
                .map(|(_, request)| request.abort)
                .collect::<Vec<_>>()
        };
        for abort in aborts {
            abort.abort();
        }
    }

    fn invalidate(&self, disable_authenticated: bool, enable_authenticated: bool) {
        let aborts = {
            let mut state = self.lock_state();
            if state.shutting_down {
                return;
            }
            state.epoch = state.epoch.wrapping_add(1);
            if disable_authenticated {
                state.authenticated_enabled = false;
            }
            if enable_authenticated {
                state.authenticated_enabled = true;
            }
            state
                .active
                .drain()
                .map(|(_, request)| request.abort)
                .collect::<Vec<_>>()
        };
        for abort in aborts {
            abort.abort();
        }
    }

    fn current_epoch(&self, scope: RequestScope) -> Result<u64, ContextError> {
        let state = self.lock_state();
        if state.shutting_down {
            return Err(ContextError::Cancelled);
        }
        if scope == RequestScope::Authenticated && !state.authenticated_enabled {
            return Err(ContextError::AuthenticationRequired);
        }
        Ok(state.epoch)
    }

    fn begin(
        &self,
        context: ExecutionContext,
    ) -> Result<(u64, futures_util::future::AbortRegistration), HttpPolicyError> {
        let (abort, registration) = AbortHandle::new_pair();
        let mut state = self.lock_state();
        if state.shutting_down
            || state.epoch != context.epoch
            || (context.scope == RequestScope::Authenticated && !state.authenticated_enabled)
        {
            return Err(HttpPolicyError::Cancelled);
        }
        state.next_request = state.next_request.wrapping_add(1);
        let request_id = state.next_request;
        state.active.insert(
            request_id,
            ActiveRequest {
                epoch: context.epoch,
                scope: context.scope,
                abort,
            },
        );
        Ok((request_id, registration))
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, PolicyState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

struct RequestGuard {
    state: Arc<Mutex<PolicyState>>,
    request_id: Option<u64>,
}

impl RequestGuard {
    fn finish(&mut self) -> bool {
        let Some(request_id) = self.request_id.take() else {
            return false;
        };
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(request) = state.active.remove(&request_id) else {
            return false;
        };
        !state.shutting_down
            && request.epoch == state.epoch
            && (request.scope != RequestScope::Authenticated || state.authenticated_enabled)
    }
}

impl Drop for RequestGuard {
    fn drop(&mut self) {
        if let Some(request_id) = self.request_id.take() {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .active
                .remove(&request_id);
        }
    }
}

#[derive(Debug)]
struct BodyLimit {
    limit: u64,
    received: u64,
}

impl BodyLimit {
    fn new(limit: u64, declared: Option<u64>) -> Result<Self, HttpPolicyError> {
        if declared.is_some_and(|length| length > limit) {
            return Err(HttpPolicyError::BodyTooLarge);
        }
        Ok(Self { limit, received: 0 })
    }

    fn accept(&mut self, chunk_bytes: usize) -> Result<(), HttpPolicyError> {
        self.received = self
            .received
            .checked_add(chunk_bytes as u64)
            .filter(|received| *received <= self.limit)
            .ok_or(HttpPolicyError::BodyTooLarge)?;
        Ok(())
    }
}

async fn read_bounded_body(
    response: reqwest::Response,
    limit: u64,
    strict: bool,
) -> Result<Vec<u8>, HttpPolicyError> {
    let declared = response.content_length();
    let mut body_limit = if strict {
        BodyLimit::new(limit, declared)?
    } else {
        BodyLimit::new(limit, None)?
    };
    let initial_capacity = declared.unwrap_or(0).min(limit).min(usize::MAX as u64) as usize;
    let mut bytes = Vec::with_capacity(initial_capacity);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            if error.is_timeout() {
                HttpPolicyError::Timeout
            } else {
                HttpPolicyError::InvalidResponse
            }
        })?;
        if body_limit.accept(chunk.len()).is_err() {
            if strict {
                return Err(HttpPolicyError::BodyTooLarge);
            }
            let remaining = limit.saturating_sub(bytes.len() as u64) as usize;
            bytes.extend_from_slice(&chunk[..remaining.min(chunk.len())]);
            break;
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn context(policy: &HttpPolicy, scope: RequestScope) -> ExecutionContext {
        ExecutionContext {
            scope,
            epoch: policy.current_epoch(scope).unwrap(),
        }
    }

    #[tokio::test]
    async fn logout_aborts_authenticated_work_and_allows_deployment_work() {
        let policy = HttpPolicy::default();
        let authenticated = context(&policy, RequestScope::Authenticated);
        let deployment = context(&policy, RequestScope::Deployment);
        let (_, authenticated_registration) = policy.begin(authenticated).unwrap();
        let (_, deployment_registration) = policy.begin(deployment).unwrap();

        policy.logout();

        assert!(Abortable::new(
            futures_util::future::pending::<()>(),
            authenticated_registration
        )
        .await
        .is_err());
        assert!(Abortable::new(
            futures_util::future::pending::<()>(),
            deployment_registration
        )
        .await
        .is_err());
        assert_eq!(
            policy.current_epoch(RequestScope::Authenticated),
            Err(ContextError::AuthenticationRequired)
        );
        assert!(policy.current_epoch(RequestScope::Deployment).is_ok());
    }

    #[test]
    fn activation_and_replacement_reject_stale_epochs() {
        let policy = HttpPolicy::default();
        let old = context(&policy, RequestScope::Authenticated);
        policy.logout();
        let deployment_epoch = policy.current_epoch(RequestScope::Deployment).unwrap();
        assert!(policy.activate_epoch(deployment_epoch));
        let current = context(&policy, RequestScope::Authenticated);

        assert_eq!(policy.begin(old).unwrap_err(), HttpPolicyError::Cancelled);
        assert!(policy.begin(current).is_ok());

        policy.deployment_replaced();
        assert_eq!(
            policy.begin(current).unwrap_err(),
            HttpPolicyError::Cancelled
        );
    }

    #[test]
    fn stale_remote_logout_cannot_disable_a_replacement_session() {
        let policy = HttpPolicy::default();
        let stale_epoch = policy.current_epoch(RequestScope::Authenticated).unwrap();
        policy.deployment_replaced();
        let current_epoch = policy.current_epoch(RequestScope::Authenticated).unwrap();

        assert!(!policy.logout_epoch(stale_epoch));
        assert!(policy.current_epoch(RequestScope::Authenticated).is_ok());
        assert!(policy.logout_epoch(current_epoch));
        assert_eq!(
            policy.current_epoch(RequestScope::Authenticated),
            Err(ContextError::AuthenticationRequired)
        );
    }

    #[tokio::test]
    async fn shutdown_is_terminal_and_idempotent() {
        let policy = HttpPolicy::default();
        let current = context(&policy, RequestScope::Deployment);
        let (_, registration) = policy.begin(current).unwrap();
        policy.shutdown();
        policy.shutdown();

        assert!(
            Abortable::new(futures_util::future::pending::<()>(), registration)
                .await
                .is_err()
        );
        assert_eq!(
            policy.current_epoch(RequestScope::Deployment),
            Err(ContextError::Cancelled)
        );
    }

    #[test]
    fn dropped_guard_removes_active_request() {
        let policy = HttpPolicy::default();
        let current = context(&policy, RequestScope::Authenticated);
        let (request_id, _) = policy.begin(current).unwrap();
        drop(RequestGuard {
            state: Arc::clone(&policy.state),
            request_id: Some(request_id),
        });
        assert!(policy.lock_state().active.is_empty());
    }

    #[test]
    fn declared_missing_and_dishonest_body_lengths_are_bounded() {
        assert_eq!(
            BodyLimit::new(4, Some(5)).unwrap_err(),
            HttpPolicyError::BodyTooLarge
        );

        let mut missing = BodyLimit::new(4, None).unwrap();
        missing.accept(2).unwrap();
        missing.accept(2).unwrap();
        assert_eq!(missing.accept(1), Err(HttpPolicyError::BodyTooLarge));

        let mut dishonest = BodyLimit::new(4, Some(1)).unwrap();
        dishonest.accept(4).unwrap();
        assert_eq!(dishonest.accept(1), Err(HttpPolicyError::BodyTooLarge));
    }

    #[test]
    fn policy_is_safe_to_share_between_commands() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<HttpPolicy>();
        let policy = Arc::new(HttpPolicy::default());
        assert_eq!(Arc::strong_count(&policy), 1);
    }
}
