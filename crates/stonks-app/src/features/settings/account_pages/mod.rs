//! The settings host owns navigation and scrolling. Mount a new page per
//! authenticated session; keep the entity across tab switches to keep drafts.
use crate::runtime::spawn_core;
use futures_util::future::{AbortHandle, Abortable};
use gpui_kit::{
    component::input::{InputEvent, InputState, TextareaState},
    *,
};
use std::{future::Future, rc::Rc};
use stonks_core::{
    account_settings::{
        AccountInfo, AccountManagement, AccountSettingsService, ProfileDraft, Session,
    },
    connection::ClientUser,
    error::CommandError,
};
use stonks_ui::{
    components::{
        input_field, settings_row, settings_section, Button, ButtonVariant, Feedback, FeedbackTone,
        SettingsInput,
    },
    i18n::{t, t_vars},
    Theme,
};

mod account;
mod privacy;
mod profile;
mod sessions;

#[derive(Clone, Copy)]
pub enum AccountPageKind {
    Profile,
    Account,
    Sessions,
    Privacy,
}

pub enum AccountPageEvent {
    ProfileSaved,
    UsernameChanged,
    UserUnblocked(ClientUser),
}

/// Optional host-owned preferences. Only supply this when the host can persist
/// changes and apply them where supported. The callback persists before
/// this page changes its displayed snapshot.
#[derive(Clone, Copy)]
pub struct PrivacyPreferences {
    pub show_notification_content: bool,
    pub send_typing_indicators: bool,
    pub display_embeds: bool,
    pub warn_external_links: bool,
}
#[derive(Clone, Copy)]
pub enum PrivacyChange {
    ShowNotificationContent(bool),
    TypingIndicators(bool),
    DisplayEmbeds(bool),
    LinkWarnings(bool),
    ClearTrustedDomains,
}

pub type PrivacyWriter = Rc<dyn Fn(PrivacyChange, &mut Window, &mut App) -> Result<(), String>>;

pub type AccountHostAction = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct AccountPage {
    kind: AccountPageKind,
    service: AccountSettingsService,
    data: Option<Data>,
    pending: Option<Pending>,
    image_picker: Option<Task<()>>,
    error: Option<String>,
    notice: Option<String>,
    editor: Option<Editor>,
    email_revealed: bool,
    open_sessions: Option<AccountHostAction>,
    account_ended: Option<AccountHostAction>,
    ended: bool,
    privacy: Option<(PrivacyPreferences, PrivacyWriter)>,
}
impl EventEmitter<AccountPageEvent> for AccountPage {}

enum Data {
    Profile(profile::Form),
    Account(ClientUser, AccountInfo),
    Sessions(Vec<Session>),
    Privacy(Vec<ClientUser>),
}
struct Pending(AbortHandle, #[allow(dead_code)] Task<()>);
impl Drop for Pending {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Clone)]
enum Action {
    Username,
    Email,
    Password,
    Rename(Session),
    Revoke(Session),
    RevokeOthers,
    Manage(AccountManagement),
}
struct Editor {
    action: Action,
    value: Option<Entity<InputState>>,
    password: Option<Entity<InputState>>,
    verification: Option<Verification>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}
struct Verification {
    methods: &'static [&'static str],
    index: usize,
    input: Entity<InputState>,
}
impl Verification {
    fn new(
        methods: &'static [&'static str],
        window: &mut Window,
        cx: &mut Context<AccountPage>,
    ) -> Self {
        Self {
            methods,
            index: 0,
            input: cx.new(|cx| InputState::new(window, cx)),
        }
    }
    fn value(&self, cx: &App) -> (String, String) {
        (
            self.methods[self.index].into(),
            self.input.read(cx).value().to_string(),
        )
    }
    fn render(&self, page: &AccountPage, theme: Theme, cx: &mut Context<AccountPage>) -> Div {
        let label = |method| {
            t(match method {
                "totp" => "settings.mfa.methods.totp",
                "recovery" => "settings.mfa.methods.recovery",
                _ => "settings.account.currentPassword",
            })
        };
        let mut section = div().flex().flex_col().gap(px(theme.space2));
        if self.methods.len() > 1 {
            section = section.child(div().flex().gap(px(theme.space2)).children(
                self.methods.iter().enumerate().map(|(index, method)| {
                    Button::new(("verification-method", index), theme)
                        .variant(if index == self.index {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Secondary
                        })
                        .label(label(*method))
                        .disabled(page.pending.is_some())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if this.pending.is_some() {
                                return;
                            }
                            if let Some(v) =
                                this.editor.as_mut().and_then(|e| e.verification.as_mut())
                            {
                                v.index = index;
                                v.input
                                    .update(cx, |input, cx| input.set_value("", window, cx));
                                cx.notify();
                            }
                        }))
                }),
            ));
        }
        let title = t(&format!("settings.mfa.fields.{}", self.methods[self.index]));
        section.child(input_field(
            &theme,
            &title,
            "",
            SettingsInput::new(&self.input, theme)
                .label(title.clone())
                .password(true)
                .disabled(page.pending.is_some()),
        ))
    }
}
impl AccountPage {
    pub fn new(
        kind: AccountPageKind,
        service: AccountSettingsService,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut page = Self {
            kind,
            service,
            data: None,
            pending: None,
            image_picker: None,
            error: None,
            notice: None,
            editor: None,
            email_revealed: false,
            open_sessions: None,
            account_ended: None,
            ended: false,
            privacy: None,
        };
        page.load(window, cx);
        page
    }
    /// The completion callback must tear down the authenticated session locally.
    pub fn set_account_actions(
        &mut self,
        open_sessions: AccountHostAction,
        account_ended: AccountHostAction,
    ) {
        self.open_sessions = Some(open_sessions);
        self.account_ended = Some(account_ended);
    }

    pub fn set_privacy_preferences(
        &mut self,
        preferences: PrivacyPreferences,
        write: PrivacyWriter,
    ) {
        self.privacy = Some((preferences, write));
    }
    /// A request can already have reached the server. Do not offer “discard”
    /// as a rollback while a mutation is in flight.
    pub fn is_busy(&self) -> bool {
        self.pending.is_some() || self.image_picker.is_some()
    }
    pub fn is_dirty(&self, cx: &App) -> bool {
        self.editor.is_some() || matches!(&self.data, Some(Data::Profile(form)) if form.dirty(cx))
    }
    fn run<T: Send + 'static>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        future: impl Future<Output = Result<T, CommandError>> + Send + 'static,
        complete: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
    ) {
        if self.pending.is_some() {
            return;
        }
        self.error = None;
        self.notice = None;
        let (abort, registration) = AbortHandle::new_pair();
        let task = cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = spawn_core(Abortable::new(future, registration)).await else {
                return;
            };
            this.update_in(cx, |this, window, cx| {
                this.pending = None;
                match result {
                    Ok(value) => complete(this, value, window, cx),
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            })
            .ok();
        });
        self.pending = Some(Pending(abort, task));
        cx.notify();
    }
    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        enum Loaded {
            Profile(ProfileDraft, ClientUser, profile::Images, bool),
            Account(ClientUser, AccountInfo),
            Sessions(Vec<Session>),
            Privacy(Vec<ClientUser>),
        }
        let kind = self.kind;
        let service = self.service.clone();
        self.run(
            window,
            cx,
            async move {
                Ok(match kind {
                    AccountPageKind::Profile => {
                        let user = service.me().await?;
                        let profile = service.profile(&user.id).await?;
                        let images = profile::Images::load(&service, &user, &profile).await;
                        Loaded::Profile(
                            ProfileDraft::from_user(&user, &profile),
                            user,
                            images,
                            profile.background.is_some(),
                        )
                    }
                    AccountPageKind::Account => {
                        Loaded::Account(service.me().await?, service.account().await?)
                    }
                    AccountPageKind::Sessions => Loaded::Sessions(service.sessions().await?),
                    AccountPageKind::Privacy => Loaded::Privacy(service.blocked_users().await?),
                })
            },
            |this, loaded, window, cx| {
                this.data = Some(match loaded {
                    Loaded::Profile(draft, user, images, background) => Data::Profile(
                        profile::Form::new(draft, user, images, background, window, cx),
                    ),
                    Loaded::Account(user, info) => Data::Account(user, info),
                    Loaded::Sessions(sessions) => Data::Sessions(sessions),
                    Loaded::Privacy(users) => Data::Privacy(users),
                });
            },
        );
    }
    fn cancel_editor(&mut self, cx: &mut Context<Self>) {
        if self.pending.is_none() {
            self.editor = None;
            self.error = None;
            cx.notify();
        }
    }
}
impl Render for AccountPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(cx);
        let mut body = div()
            .flex()
            .flex_col()
            .min_w_0()
            .gap(px(if matches!(self.kind, AccountPageKind::Privacy) {
                0.
            } else {
                theme.space3
            }))
            .text_color(theme.text);
        if let Some(error) = &self.error {
            body = body.child(Feedback::new(theme, FeedbackTone::Error, error.clone()));
        }
        if let Some(notice) = &self.notice {
            body = body.child(Feedback::new(theme, FeedbackTone::Success, notice.clone()));
        }
        if matches!(self.kind, AccountPageKind::Privacy) {
            body = body.child(self.privacy_preferences(theme, cx));
        }
        if self.ended {
            return body;
        }
        let content = match &self.data {
            Some(Data::Profile(form)) => self.profile(form, theme, window, cx),
            Some(Data::Account(user, info)) => self.account(user, info, theme, cx),
            Some(Data::Sessions(rows)) => self.sessions(rows, theme, cx),
            Some(Data::Privacy(users)) => self.blocked_users(users, theme, cx),
            None if self.pending.is_some() => div().child(t("common.loading")),
            None => div().child(
                Button::new("account-page-retry", theme)
                    .label(t("common.retry"))
                    .on_click(cx.listener(Self::retry)),
            ),
        };
        body = body.child(content);
        if let Some(editor) = &self.editor {
            body = body.child(
                deferred(
                    anchored().position(point(px(0.), px(0.))).child(
                        div()
                            .relative()
                            .w(window.viewport_size().width)
                            .h(window.viewport_size().height)
                            .child(self.editor(editor, theme, cx)),
                    ),
                )
                .with_priority(100),
            );
        }
        body
    }
}
impl AccountPage {
    fn retry(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.load(window, cx);
    }
}

fn subtitle(theme: Theme, key: &str, small: bool) -> Div {
    div()
        .text_size(px(if small {
            theme.font_size_xs
        } else {
            theme.font_size_sm
        }))
        .text_color(theme.text_muted)
        .child(t(key))
}
