//! Rendering for the auth screen — one function per view, mirroring
//! `src/features/auth/components/LoginScreen.tsx` and `Auth.stylex.ts`.
//! All state and actions live in `screen.rs`; these are pure builders.

use gpui_kit::component::input::{Input, InputContentType};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use stonks_core::auth::models::MfaMethod;
use stonks_ui::components::{Button, ButtonVariant, Checkbox, Feedback, FeedbackTone};
use stonks_ui::i18n::t;
use stonks_ui::Theme;

use super::screen::{AuthScreen, AuthView};

pub fn heading(theme: &Theme, title: String, description: String) -> Div {
    div()
        .mb(px(theme.space4))
        .child(
            div()
                .text_size(px(28.))
                .font_weight(FontWeight::BOLD)
                .mb(px(theme.space3))
                .child(title),
        )
        .child(
            div()
                .text_size(px(theme.font_size_md))
                .text_color(theme.text_muted)
                .child(description),
        )
}

pub fn heading_for(view: AuthView, server_name: Option<&str>) -> (String, String) {
    match view {
        AuthView::Login => (format!("👋 {}", t("auth.welcomeBack")), t("auth.signIn")),
        AuthView::Mfa => (t("auth.mfaTitle"), t("auth.mfaDescription")),
        AuthView::Signup => (
            t("auth.createAccount"),
            t("auth.joinServer").replace("{server}", server_name.unwrap_or("Stoat")),
        ),
        AuthView::Reset => (t("auth.resetPassword"), t("auth.enterEmail")),
        AuthView::Reverify => (t("auth.verifyEmail"), t("auth.enterEmail")),
        AuthView::Verify => (t("auth.verifyEmail"), t("auth.enterEmailCode")),
        AuthView::ResetConfirm => (t("auth.newPasswordTitle"), t("auth.useResetToken")),
        AuthView::Onboarding => (t("auth.chooseUsername"), t("auth.finishAccount")),
        AuthView::Welcome => unreachable!("welcome renders separately"),
    }
}

pub fn welcome(theme: &Theme, cx: &mut Context<AuthScreen>) -> Div {
    div()
        .flex()
        .flex_col()
        .w(px(380.))
        .child(
            div()
                .mb(px(3.))
                .text_sm()
                .text_color(theme.text_muted)
                .child("Stonks Chat"),
        )
        .child(heading(
            theme,
            t("auth.welcomeTitle"),
            t("auth.welcomeDescription"),
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    Button::new("open-login", *theme)
                        .variant(ButtonVariant::Primary)
                        .label(t("auth.login"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open_view(AuthView::Login, cx);
                        })),
                )
                .child(
                    Button::new("open-signup", *theme)
                        .variant(ButtonVariant::Secondary)
                        .label(t("auth.signup"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open_view(AuthView::Signup, cx);
                        })),
                ),
        )
        .child(feedback(theme, None, None))
}

pub fn signed_in(
    theme: &Theme,
    username: &str,
    busy: bool,
    cx: &mut Context<AuthScreen>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .items_center()
        .child(
            div()
                .text_size(px(32.))
                .text_color(theme.success)
                .child("✓"),
        )
        .child(
            div()
                .text_size(px(24.))
                .font_weight(FontWeight::BOLD)
                .child(t("auth.welcomeUser").replace("{user}", username)),
        )
        .child(
            Button::new("logout", *theme)
                .variant(ButtonVariant::Secondary)
                .label(t("settings.logout"))
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| this.logout(cx))),
        )
}

fn feedback(theme: &Theme, notice: Option<String>, error: Option<String>) -> Div {
    let mut column = div().flex().flex_col();
    if let Some(notice) = notice {
        column = column.child(Feedback::new(*theme, FeedbackTone::Success, notice));
    }
    if let Some(error) = error {
        column = column.child(Feedback::new(*theme, FeedbackTone::Error, error));
    }
    column
}

fn captcha_warning(theme: &Theme, blocked: bool) -> Option<Feedback> {
    blocked.then(|| {
        Feedback::new(
            *theme,
            FeedbackTone::Error,
            "This server requires captcha, which the GPUI build cannot solve yet (M7). Use the desktop app for this step.",
        )
    })
}

fn back(theme: &Theme, target: AuthView, cx: &mut Context<AuthScreen>) -> Button {
    Button::new("back", *theme)
        .variant(ButtonVariant::Quiet)
        .label(format!("← {}", t("auth.back")))
        .on_click(cx.listener(move |this, _, _, cx| this.open_view(target, cx)))
}

fn submit(
    theme: &Theme,
    label: String,
    busy_label: String,
    disabled: bool,
    cx: &mut Context<AuthScreen>,
) -> Button {
    let screen = cx.entity().read(cx);
    let busy = screen.busy;
    Button::new("submit", *theme)
        .variant(ButtonVariant::Primary)
        .label(if busy { busy_label } else { label })
        .disabled(disabled || busy)
        .on_click(cx.listener(|this, _, _, cx| {
            this.submit(cx);
        }))
}

fn actions(left: Button, right: Button) -> Div {
    div().flex().justify_between().child(left).child(right)
}

pub fn form(screen: &mut AuthScreen, theme: &Theme, cx: &mut Context<AuthScreen>) -> AnyElement {
    let busy = screen.busy;
    let notice = screen.notice.clone();
    let error = screen.error.clone();
    match screen.view {
        AuthView::Login | AuthView::Mfa => {
            let mfa = screen.view == AuthView::Mfa;
            let mut form = div().flex().flex_col().gap_3();
            if mfa {
                let methods = screen.mfa_methods.clone();
                let method = screen.mfa_method;
                form = form
                    .child(Input::new(&screen.mfa_code))
                    .when(
                        methods.iter().any(|m| m == "Recovery") && method != MfaMethod::Recovery,
                        |form| {
                            form.child(
                                Button::new("use-recovery", *theme)
                                    .variant(ButtonVariant::Quiet)
                                    .label(t("auth.useRecovery"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.mfa_method = MfaMethod::Recovery;
                                        cx.notify();
                                    })),
                            )
                        },
                    )
                    .when(
                        methods.iter().any(|m| m == "Totp") && method == MfaMethod::Recovery,
                        |form| {
                            form.child(
                                Button::new("use-totp", *theme)
                                    .variant(ButtonVariant::Quiet)
                                    .label(t("auth.useAuthenticator"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.mfa_method = MfaMethod::Totp;
                                        cx.notify();
                                    })),
                            )
                        },
                    );
            } else {
                form = form
                    .child(Input::new(&screen.email))
                    .child(Input::new(&screen.password).content_type(InputContentType::Password))
                    .child(
                        div()
                            .flex()
                            .child(
                                Button::new("link-reset", *theme)
                                    .variant(ButtonVariant::Quiet)
                                    .label(t("auth.resetPassword"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.open_view(AuthView::Reset, cx);
                                    })),
                            )
                            .child(
                                Button::new("link-reverify", *theme)
                                    .variant(ButtonVariant::Quiet)
                                    .label(t("auth.resendVerification"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.open_view(AuthView::Reverify, cx);
                                    })),
                            ),
                    );
            }
            form.child(feedback(theme, notice, error))
                .child(actions(
                    back(theme, AuthView::Welcome, cx),
                    submit(
                        theme,
                        if mfa { t("auth.verify") } else { t("auth.login") },
                        t("auth.wait"),
                        false,
                        cx,
                    ),
                ))
                .into_any_element()
        }
        AuthView::Signup => {
            let invite_only = screen
                .server
                .as_ref()
                .is_some_and(|server| server.capabilities.invite_only);
            let captcha = screen.captcha_blocked();
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(Input::new(&screen.email))
                .when(invite_only, |form| form.child(Input::new(&screen.invite)))
                .child(Input::new(&screen.password).content_type(InputContentType::Password))
                .child(
                    Input::new(&screen.confirm_password).content_type(InputContentType::Password),
                )
                .when_some(captcha_warning(theme, captcha), |form, warning| {
                    form.child(warning)
                })
                .child(feedback(theme, notice, error))
                .child(actions(
                    back(theme, AuthView::Welcome, cx),
                    submit(theme, t("auth.createAccount"), t("auth.creating"), captcha, cx),
                ))
                .into_any_element()
        }
        AuthView::Reset | AuthView::Reverify => {
            let captcha = screen.captcha_blocked();
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(Input::new(&screen.email))
                .when_some(captcha_warning(theme, captcha), |form, warning| {
                    form.child(warning)
                })
                .child(feedback(theme, notice, error))
                .child(actions(
                    back(theme, AuthView::Login, cx),
                    submit(theme, t("auth.sendEmail"), t("auth.sending"), captcha, cx),
                ))
                .into_any_element()
        }
        AuthView::Verify => div()
            .flex()
            .flex_col()
            .gap_3()
            .child(Input::new(&screen.code))
            .child(feedback(theme, notice, error))
            .child(actions(
                back(theme, AuthView::Login, cx),
                submit(theme, t("auth.verify"), t("common.verifying"), false, cx),
            ))
            .into_any_element(),
        AuthView::ResetConfirm => div()
            .flex()
            .flex_col()
            .gap_3()
            .child(Input::new(&screen.code))
            .child(Input::new(&screen.password).content_type(InputContentType::Password))
            .child(Input::new(&screen.confirm_password).content_type(InputContentType::Password))
            .child(
                Checkbox::new("remove-sessions", *theme)
                    .label(t("auth.signOutOthers"))
                    .checked(screen.remove_sessions)
                    .on_change(cx.listener(|this, checked, _, cx| {
                        this.remove_sessions = *checked;
                        cx.notify();
                    })),
            )
            .child(feedback(theme, notice, error))
            .child(actions(
                back(theme, AuthView::Login, cx),
                submit(theme, t("auth.updatePassword"), t("auth.updating"), false, cx),
            ))
            .into_any_element(),
        AuthView::Onboarding => div()
            .flex()
            .flex_col()
            .gap_3()
            .child(Input::new(&screen.username))
            .child(feedback(theme, notice, error))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .child(
                        Button::new("onboarding-logout", *theme)
                            .variant(ButtonVariant::Quiet)
                            .label(t("settings.logout"))
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| this.logout(cx))),
                    )
                    .child(submit(theme, t("auth.continue"), t("auth.saving"), false, cx)),
            )
            .into_any_element(),
        AuthView::Welcome => unreachable!("welcome renders separately"),
    }
}
