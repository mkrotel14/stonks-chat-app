//! The authentication screen entity: flow state, async actions against
//! `stonks-core`, and session restore. Rendering lives in `views.rs`;
//! the root `AppView` hosts this screen and reacts to its events.

use std::future::Future;
use std::sync::Arc;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use stonks_core::auth::models::{
    EmailActionRequest, LoginRequest, LoginResult, MfaMethod, OnboardingRequest,
    PasswordResetRequest, SessionState, SignupRequest,
};
use stonks_core::auth::service;
use stonks_core::error::CommandError;
use stonks_core::http::HttpPolicy;
use stonks_core::server::ServerSelection;
use stonks_ui::i18n::t;
use stonks_ui::Theme;

use crate::runtime::spawn_core;

use super::views;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AuthView {
    Welcome,
    Login,
    Mfa,
    Signup,
    Reset,
    Reverify,
    Verify,
    ResetConfirm,
    Onboarding,
}

pub enum AuthOutcome {
    Login(LoginResult),
    Session(SessionState),
    Unit,
}

/// Events the root view reacts to.
pub enum AuthScreenEvent {
    OpenServerPicker,
    /// A session exists (login, restore or onboarding completed).
    Authenticated,
    /// Restore finished without a session ready for the shell.
    RestoreNeedsAuth,
    /// The user logged out or the session was dropped.
    SignedOut,
}

pub struct AuthScreen {
    pub policy: Arc<HttpPolicy>,
    pub server: Option<ServerSelection>,
    pub view: AuthView,
    pub restoring: bool,
    pub email: Entity<InputState>,
    pub password: Entity<InputState>,
    pub confirm_password: Entity<InputState>,
    pub invite: Entity<InputState>,
    pub code: Entity<InputState>,
    pub mfa_code: Entity<InputState>,
    pub username: Entity<InputState>,
    pub remove_sessions: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub mfa_ticket: Option<String>,
    pub mfa_method: MfaMethod,
    pub mfa_methods: Vec<String>,
    pub session: Option<(String, String)>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<AuthScreenEvent> for AuthScreen {}

impl AuthScreen {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        server: Option<ServerSelection>,
        policy: Arc<HttpPolicy>,
    ) -> Self {
        let input = |window: &mut Window, cx: &mut Context<Self>, placeholder: String| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let email = input(window, cx, t("auth.email"));
        let password = input(window, cx, t("auth.password"));
        let confirm_password = input(window, cx, t("auth.confirmPassword"));
        let invite = input(window, cx, t("auth.inviteCode"));
        let code = input(window, cx, t("auth.verificationCode"));
        let mfa_code = input(window, cx, t("auth.authenticationCode"));
        let username = input(window, cx, t("auth.username"));

        let mut subscriptions = Vec::new();
        for state in [
            &email,
            &password,
            &confirm_password,
            &invite,
            &code,
            &mfa_code,
            &username,
        ] {
            subscriptions.push(cx.subscribe_in(state, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.submit(cx);
                }
            }));
        }

        Self {
            policy,
            server,
            view: AuthView::Welcome,
            restoring: false,
            email,
            password,
            confirm_password,
            invite,
            code,
            mfa_code,
            username,
            remove_sessions: true,
            busy: false,
            error: None,
            notice: None,
            mfa_ticket: None,
            mfa_method: MfaMethod::Totp,
            mfa_methods: Vec::new(),
            session: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn server_display_name(&self) -> Option<&str> {
        self.server
            .as_ref()
            .map(|server| server.display_name.as_str())
    }

    pub fn set_server(&mut self, server: ServerSelection, cx: &mut Context<Self>) {
        self.policy.deployment_replaced();
        self.server = Some(server);
        self.restore_session(cx);
    }

    pub fn restore_session(&mut self, cx: &mut Context<Self>) {
        if self.restoring {
            return;
        }
        self.restoring = true;
        cx.notify();
        let policy = self.policy.clone();
        cx.spawn(async move |this, cx| {
            let result = spawn_core(async move { service::load_session(&policy).await }).await;
            this.update(cx, |this, cx| {
                this.restoring = false;
                match result {
                    Ok(Some(SessionState::Authenticated { user_id, username })) => {
                        this.session = Some((user_id, username));
                        cx.emit(AuthScreenEvent::Authenticated);
                    }
                    Ok(Some(SessionState::Onboarding)) => {
                        this.view = AuthView::Onboarding;
                        cx.emit(AuthScreenEvent::RestoreNeedsAuth);
                    }
                    Ok(None) | Err(_) => cx.emit(AuthScreenEvent::RestoreNeedsAuth),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn open_view(&mut self, view: AuthView, cx: &mut Context<Self>) {
        self.view = view;
        self.error = None;
        self.notice = None;
        cx.notify();
    }

    pub fn open_server_picker(&mut self, cx: &mut Context<Self>) {
        if !self.restoring && !self.busy {
            cx.emit(AuthScreenEvent::OpenServerPicker);
        }
    }

    pub(crate) fn captcha_blocked(&self) -> bool {
        // Captcha is an M7 item (webview-backed); the GPUI build cannot
        // produce tokens yet, so captcha-gated operations would fail
        // server-side.
        self.server
            .as_ref()
            .is_some_and(|server| server.capabilities.captcha.enabled)
    }

    pub(crate) fn submit(&mut self, cx: &mut Context<Self>) {
        if self.restoring || self.busy || self.session.is_some() {
            return;
        }
        match self.view {
            AuthView::Login | AuthView::Mfa => self.submit_login(cx),
            AuthView::Signup => self.submit_signup(cx),
            AuthView::Reset | AuthView::Reverify => self.submit_email_action(cx),
            AuthView::Verify => self.submit_verification(cx),
            AuthView::ResetConfirm => self.submit_reset(cx),
            AuthView::Onboarding => self.submit_onboarding(cx),
            AuthView::Welcome => {}
        }
    }

    fn value(cx: &Context<Self>, state: &Entity<InputState>) -> String {
        state.read(cx).value().to_string()
    }

    fn run<F, R>(&mut self, cx: &mut Context<Self>, action: F, done: R)
    where
        F: Future<Output = Result<AuthOutcome, CommandError>> + Send + 'static,
        R: FnOnce(&mut Self, Result<AuthOutcome, String>, &mut Context<Self>) + 'static,
    {
        self.busy = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = spawn_core(action).await.map_err(|error| error.to_string());
            this.update(cx, |this, cx| {
                this.busy = false;
                done(this, result, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn submit_login(&mut self, cx: &mut Context<Self>) {
        let request = if self.view == AuthView::Mfa {
            LoginRequest {
                email: None,
                password: None,
                mfa_ticket: self.mfa_ticket.clone(),
                mfa_code: Some(Self::value(cx, &self.mfa_code)),
                mfa_method: Some(self.mfa_method),
            }
        } else {
            LoginRequest {
                email: Some(Self::value(cx, &self.email)),
                password: Some(Self::value(cx, &self.password)),
                mfa_ticket: None,
                mfa_code: None,
                mfa_method: None,
            }
        };
        let policy = self.policy.clone();
        self.run(
            cx,
            async move {
                service::login(&policy, request)
                    .await
                    .map(AuthOutcome::Login)
            },
            |this, result, cx| match result {
                Ok(AuthOutcome::Login(LoginResult::Authenticated { user_id, username })) => {
                    this.session = Some((user_id, username));
                    cx.emit(AuthScreenEvent::Authenticated);
                }
                Ok(AuthOutcome::Login(LoginResult::Mfa {
                    ticket,
                    allowed_methods,
                })) => {
                    this.mfa_ticket = Some(ticket);
                    this.mfa_method = if allowed_methods.iter().any(|m| m == "Totp") {
                        MfaMethod::Totp
                    } else {
                        MfaMethod::Recovery
                    };
                    this.mfa_methods = allowed_methods;
                    this.view = AuthView::Mfa;
                }
                Ok(AuthOutcome::Login(LoginResult::Onboarding)) => {
                    this.view = AuthView::Onboarding;
                }
                Ok(_) => {}
                Err(error) => this.error = Some(error),
            },
        );
    }

    fn submit_signup(&mut self, cx: &mut Context<Self>) {
        let password = Self::value(cx, &self.password);
        if password != Self::value(cx, &self.confirm_password) {
            self.error = Some("Passwords do not match.".into());
            cx.notify();
            return;
        }
        let request = SignupRequest {
            email: Self::value(cx, &self.email),
            password,
            invite: {
                let invite = Self::value(cx, &self.invite);
                (!invite.is_empty()).then_some(invite)
            },
            captcha: None,
        };
        let policy = self.policy.clone();
        let needs_verification = self
            .server
            .as_ref()
            .is_some_and(|server| server.capabilities.email_verification);
        self.run(
            cx,
            async move {
                service::create_account(&policy, request)
                    .await
                    .map(|()| AuthOutcome::Unit)
            },
            move |this, result, _cx| match result {
                Ok(_) => {
                    this.notice = Some(if needs_verification {
                        "Account created. Check your email for the verification code.".into()
                    } else {
                        "Account created. You can log in now.".into()
                    });
                    this.view = if needs_verification {
                        AuthView::Verify
                    } else {
                        AuthView::Login
                    };
                }
                Err(error) => this.error = Some(error),
            },
        );
    }

    fn submit_email_action(&mut self, cx: &mut Context<Self>) {
        let reset = self.view == AuthView::Reset;
        let request = EmailActionRequest {
            email: Self::value(cx, &self.email),
            captcha: None,
        };
        let policy = self.policy.clone();
        self.run(
            cx,
            async move {
                if reset {
                    service::send_password_reset(&policy, request).await
                } else {
                    service::resend_verification(&policy, request).await
                }
                .map(|()| AuthOutcome::Unit)
            },
            move |this, result, _cx| match result {
                Ok(_) => {
                    this.notice = Some(if reset {
                        "Reset email sent. Paste its token below.".into()
                    } else {
                        "Verification email sent. Paste its code below.".into()
                    });
                    this.view = if reset {
                        AuthView::ResetConfirm
                    } else {
                        AuthView::Verify
                    };
                }
                Err(error) => this.error = Some(error),
            },
        );
    }

    fn submit_verification(&mut self, cx: &mut Context<Self>) {
        let code = Self::value(cx, &self.code);
        let policy = self.policy.clone();
        self.run(
            cx,
            async move {
                service::verify_email(&policy, code)
                    .await
                    .map(|()| AuthOutcome::Unit)
            },
            |this, result, _cx| match result {
                Ok(_) => {
                    this.notice = Some("Email verified. You can log in now.".into());
                    this.view = AuthView::Login;
                }
                Err(error) => this.error = Some(error),
            },
        );
    }

    fn submit_reset(&mut self, cx: &mut Context<Self>) {
        let password = Self::value(cx, &self.password);
        if password != Self::value(cx, &self.confirm_password) {
            self.error = Some("Passwords do not match.".into());
            cx.notify();
            return;
        }
        let request = PasswordResetRequest {
            token: Self::value(cx, &self.code),
            password,
            remove_sessions: self.remove_sessions,
        };
        let policy = self.policy.clone();
        self.run(
            cx,
            async move {
                service::confirm_password_reset(&policy, request)
                    .await
                    .map(|()| AuthOutcome::Unit)
            },
            |this, result, _cx| match result {
                Ok(_) => {
                    this.notice = Some("Password updated. You can log in now.".into());
                    this.view = AuthView::Login;
                }
                Err(error) => this.error = Some(error),
            },
        );
    }

    fn submit_onboarding(&mut self, cx: &mut Context<Self>) {
        let request = OnboardingRequest {
            username: Self::value(cx, &self.username),
        };
        let policy = self.policy.clone();
        self.run(
            cx,
            async move {
                service::complete_onboarding(&policy, request)
                    .await
                    .map(AuthOutcome::Session)
            },
            |this, result, cx| match result {
                Ok(AuthOutcome::Session(SessionState::Authenticated { user_id, username })) => {
                    this.session = Some((user_id, username));
                    cx.emit(AuthScreenEvent::Authenticated);
                }
                Ok(_) => {}
                Err(error) => this.error = Some(error),
            },
        );
    }

    pub fn logout(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        cx.notify();
        let policy = self.policy.clone();
        cx.spawn(async move |this, cx| {
            let _ = spawn_core(async move { service::logout(&policy).await }).await;
            this.update(cx, |this, cx| {
                this.busy = false;
                this.session = None;
                this.view = AuthView::Welcome;
                this.mfa_ticket = None;
                cx.emit(AuthScreenEvent::SignedOut);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for AuthScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(cx);

        if self.restoring {
            return div()
                .text_color(theme.text_muted)
                .text_sm()
                .child(t("auth.restoring"))
                .into_any_element();
        }
        if let Some((_, username)) = &self.session {
            return views::signed_in(&theme, username, self.busy, cx).into_any_element();
        }
        match self.view {
            AuthView::Welcome => views::welcome(&theme, cx).into_any_element(),
            view => {
                let (title, description) = views::heading_for(view, self.server_display_name());
                div()
                    .flex()
                    .flex_col()
                    .w(px(380.))
                    .child(views::heading(&theme, title, description))
                    .child(views::form(self, &theme, cx))
                    .into_any_element()
            }
        }
    }
}
