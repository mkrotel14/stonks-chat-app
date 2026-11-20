use super::*;
use gpui_kit::prelude::FluentBuilder;
use std::sync::Arc;
use stonks_core::{
    account_settings::Profile, connection::ClientFileReference, uploads::UploadFile,
};

type Texture = Arc<RenderImage>;
struct Selected(Arc<UploadFile>, Texture);
pub(super) struct Images(Option<Vec<image::Frame>>, Option<Vec<image::Frame>>);
impl Images {
    pub(super) async fn load(
        service: &AccountSettingsService,
        user: &ClientUser,
        profile: &Profile,
    ) -> Self {
        async fn load(
            service: &AccountSettingsService,
            file: Option<&ClientFileReference>,
            edge: u32,
        ) -> Option<Vec<image::Frame>> {
            let bytes = service.profile_image(file?).await?;
            crate::shared::images::preview_frames(bytes, edge).await
        }
        let (avatar, background) = tokio::join!(
            load(service, user.avatar.as_ref(), 128),
            load(service, profile.background.as_ref(), 960)
        );
        Self(avatar, background)
    }
}
fn texture(frames: Vec<image::Frame>) -> Texture {
    Arc::new(RenderImage::new(frames))
}

use stonks_core::account_settings::PRESENCES;
use stonks_ui::components::{
    select::{IndexPath, SearchableVec},
    UiSelect, UiSelectEvent, UiSelectOption, UiSelectState,
};

const FIELDS: [&str; 3] = [
    "settings.profile.displayName",
    "settings.profile.pronouns",
    "settings.profile.status",
];
pub(super) struct Form {
    avatar_focus: FocusHandle,
    initial: ProfileDraft,
    fields: [Entity<InputState>; 3],
    about: Entity<TextareaState>,
    presence: Entity<UiSelectState>,
    has_background: bool,
    user: ClientUser,
    avatar: Option<Texture>,
    background: Option<Texture>,
    next_avatar: Option<Selected>,
    next_background: Option<Selected>,
    remove_background: bool,
    _subscriptions: Vec<Subscription>,
}
impl Form {
    pub(super) fn new(
        initial: ProfileDraft,
        user: ClientUser,
        images: Images,
        has_background: bool,
        window: &mut Window,
        cx: &mut Context<AccountPage>,
    ) -> Self {
        let fields = [
            (&initial.display_name, user.username.clone()),
            (&initial.pronouns, t("settings.profile.optional")),
            (&initial.status, t("settings.profile.statusPlaceholder")),
        ]
        .map(|(value, placeholder)| {
            let field = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
            field.update(cx, |field, cx| field.set_value(value.clone(), window, cx));
            field
        });
        let about = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(t("settings.profile.aboutPlaceholder"))
                .rows(6)
        });
        about.update(cx, |field, cx| {
            field.set_value(initial.about.clone(), window, cx)
        });
        let options: Vec<_> = PRESENCES
            .iter()
            .zip(["online", "idle", "focus", "busy", "invisible"])
            .map(|(value, label)| {
                UiSelectOption::new(*value, t(&format!("settings.profile.{label}")))
            })
            .collect();
        let index = PRESENCES
            .iter()
            .position(|value| *value == initial.presence)
            .unwrap_or(0);
        let presence = cx.new(|cx| {
            UiSelectState::new(
                SearchableVec::new(options),
                Some(IndexPath::new(index)),
                window,
                cx,
            )
        });
        let mut subscriptions: Vec<_> = fields
            .iter()
            .map(|field| cx.subscribe(field, changed))
            .collect();
        subscriptions.push(cx.subscribe(&about, |page, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                page.notice = None;
                cx.notify();
            }
        }));
        subscriptions.push(cx.subscribe(&presence, |page, _, event, cx| {
            if matches!(event, UiSelectEvent::Confirm(_)) {
                page.notice = None;
                cx.notify();
            }
        }));
        Self {
            avatar_focus: cx.focus_handle(),
            initial,
            fields,
            about,
            presence,
            has_background,
            user,
            avatar: images.0.map(texture),
            background: images.1.map(texture),
            next_avatar: None,
            next_background: None,
            remove_background: false,
            _subscriptions: subscriptions,
        }
    }
    fn draft(&self, cx: &App) -> ProfileDraft {
        ProfileDraft {
            display_name: self.fields[0].read(cx).value().to_string(),
            pronouns: self.fields[1].read(cx).value().to_string(),
            status: self.fields[2].read(cx).value().to_string(),
            presence: self
                .presence
                .read(cx)
                .selected_value()
                .map(ToString::to_string)
                .unwrap_or_else(|| self.initial.presence.clone()),
            about: self.about.read(cx).value().to_string(),
            remove_background: self.remove_background,
        }
    }
    pub(super) fn dirty(&self, cx: &App) -> bool {
        self.draft(cx) != self.initial
            || self.next_avatar.is_some()
            || self.next_background.is_some()
    }
}
fn changed(
    page: &mut AccountPage,
    _: Entity<InputState>,
    event: &InputEvent,
    cx: &mut Context<AccountPage>,
) {
    if matches!(event, InputEvent::Change) {
        page.notice = None;
        cx.notify();
    }
}
impl AccountPage {
    pub(super) fn profile(
        &self,
        form: &Form,
        theme: Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let busy = self.is_busy();
        let avatar = form
            .next_avatar
            .as_ref()
            .map(|s| &s.1)
            .or(form.avatar.as_ref());
        let background = form
            .next_background
            .as_ref()
            .map(|s| &s.1)
            .or(form.background.as_ref())
            .filter(|_| !form.remove_background);
        let has_background =
            !form.remove_background && (form.has_background || form.next_background.is_some());
        let display = form.fields[0].read(cx).value();
        let name = if display.trim().is_empty() {
            &form.user.username
        } else {
            display.trim()
        };
        let handle = form
            .user
            .discriminator
            .as_ref()
            .filter(|tag| !tag.is_empty())
            .map_or_else(
                || form.user.username.clone(),
                |tag| format!("{}#{tag}", form.user.username),
            );
        let avatar_label = t("settings.profile.changePicture");
        let mut badge = div()
            .id("profile-avatar")
            .group("profile-avatar")
            .role(Role::Button)
            .aria_label(avatar_label.clone())
            .relative()
            .size(px(64.))
            .rounded_full()
            .overflow_hidden()
            .flex_shrink_0()
            .bg(theme.surface_interactive)
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(theme.font_size_sm))
            .font_weight(FontWeight::BOLD);
        badge = if let Some(image) = avatar {
            badge.child(
                img(image.clone())
                    .size_full()
                    .rounded_full()
                    .object_fit(ObjectFit::Cover),
            )
        } else {
            badge.child(
                name.chars()
                    .next()
                    .map(|c| c.to_uppercase().to_string())
                    .unwrap_or_default(),
            )
        };
        badge = if busy {
            badge.cursor_not_allowed()
        } else {
            let focused = form.avatar_focus.is_focused(window);
            badge
                .track_focus(&form.avatar_focus)
                .tab_index(0)
                .cursor_pointer()
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(avatar_label.clone())
                        .build(window, cx)
                })
                .on_click(
                    cx.listener(|this, _, window, cx| this.pick_profile_image(false, window, cx)),
                )
                .child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .py(px(6.))
                        .px(px(2.))
                        .bg(theme.surface_raised)
                        .text_center()
                        .text_size(px(theme.font_size_xs))
                        .font_weight(FontWeight::NORMAL)
                        .opacity(if focused { 1. } else { 0. })
                        .group_hover("profile-avatar", |style| style.opacity(1.))
                        .child(t("settings.profile.change")),
                )
        };
        let identity = div()
            .flex()
            .min_w_0()
            .items_center()
            .flex_wrap()
            .gap(px(17.))
            .mt(px(theme.space4))
            .mb(px(theme.space6))
            .p(px(theme.space5))
            .rounded(px(theme.radius_lg))
            .bg(theme.surface_raised)
            .child(badge)
            .child(
                div()
                    .flex_grow(1.)
                    .flex_basis(px(160.))
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(theme.font_size_lg))
                            .font_weight(FontWeight::BOLD)
                            .mb(px(theme.space1))
                            .child(name.to_owned()),
                    )
                    .child(
                        div()
                            .text_size(px(theme.font_size_sm))
                            .text_color(theme.text_muted)
                            .child(handle),
                    ),
            );
        let banner = div()
            .w_full()
            .h(px(150.))
            .rounded(px(theme.radius_md))
            .border_1()
            .border_color(theme.outline)
            .overflow_hidden()
            .bg(theme.surface_interactive)
            .flex()
            .items_center()
            .justify_center();
        let banner = if let Some(image) = background {
            banner.child(img(image.clone()).size_full().object_fit(ObjectFit::Cover))
        } else if has_background {
            banner
        } else {
            banner.child(subtitle(theme, "settings.profile.noBackground", false))
        };
        let background_actions = div()
            .flex()
            .flex_wrap()
            .min_w_0()
            .items_center()
            .justify_between()
            .gap(px(theme.space3))
            .child(
                div()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(theme.font_size_sm))
                            .font_weight(FontWeight::BOLD)
                            .child(t("settings.profile.background")),
                    )
                    .child(
                        div()
                            .mt(px(theme.space1))
                            .text_size(px(theme.font_size_xs))
                            .text_color(theme.text_muted)
                            .child(t("settings.profile.backgroundHint")),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(theme.space2))
                    .child(
                        Button::new("profile-background-select", theme)
                            .variant(ButtonVariant::Secondary)
                            .label(t(if has_background {
                                "settings.profile.changeBackground"
                            } else {
                                "settings.profile.addBackground"
                            }))
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.pick_profile_image(true, window, cx)
                            })),
                    )
                    .when(has_background || form.remove_background, |row| {
                        row.child(
                            Button::new("profile-background-remove", theme)
                                .variant(ButtonVariant::DangerQuiet)
                                .label(t(if form.remove_background {
                                    "common.cancel"
                                } else {
                                    "settings.profile.removeBackground"
                                }))
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if this.is_busy() {
                                        return;
                                    }
                                    if let Some(Data::Profile(form)) = &mut this.data {
                                        form.next_background = None;
                                        form.remove_background =
                                            form.has_background && !form.remove_background;
                                        this.notice = None;
                                        cx.notify();
                                    }
                                })),
                        )
                    }),
            );
        let field = |index: usize| {
            let label = t(FIELDS[index]);
            input_field(
                &theme,
                &label,
                "",
                SettingsInput::new(&form.fields[index], theme)
                    .label(label.clone())
                    .disabled(busy),
            )
        };
        let form_column = || {
            div()
                .flex_grow(1.)
                .flex_basis(px(220.))
                .min_w_0()
                .max_w_full()
        };
        let presence = t("settings.profile.presence");
        let about = t("settings.profile.about");
        let labeled = |label: String, control: AnyElement| {
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .gap(px(7.))
                .child(
                    div()
                        .text_color(theme.text_muted)
                        .text_size(px(theme.font_size_sm))
                        .font_weight(FontWeight::BOLD)
                        .child(label),
                )
                .child(control)
        };
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .child(identity)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme.space3))
                    .mb(px(theme.space5))
                    .child(banner)
                    .child(background_actions),
            )
            .child(
                div()
                    .text_color(theme.text)
                    .text_size(px(theme.font_size_lg))
                    .font_weight(FontWeight::BOLD)
                    .mb(px(theme.space3))
                    .child(t("settings.profile.edit")),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(15.))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(15.))
                            .children([0, 1].map(|i| form_column().child(field(i)))),
                    )
                    .child(field(2))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(15.))
                            .gap_y(px(0.))
                            .child(
                                form_column().child(labeled(
                                    presence.clone(),
                                    UiSelect::new(&form.presence, theme)
                                        .label(presence)
                                        .disabled(busy)
                                        .into_any_element(),
                                )),
                            )
                            // The empty second column wraps away at the same breakpoint as the text fields.
                            .child(form_column()),
                    )
                    .child(labeled(
                        about.clone(),
                        SettingsInput::textarea(&form.about, theme)
                            .textarea_rows(6)
                            .label(about)
                            .disabled(busy)
                            .into_any_element(),
                    )),
            )
            .child(
                div().flex().justify_end().mt(px(theme.space4)).child(
                    Button::new("profile-save", theme)
                        .label(t(if self.pending.is_some() {
                            "settings.profile.saving"
                        } else {
                            "settings.profile.save"
                        }))
                        .disabled(busy || !form.dirty(cx))
                        .on_click(cx.listener(|this, _, window, cx| this.save_profile(window, cx))),
                ),
            )
    }
    fn pick_profile_image(
        &mut self,
        background: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_busy() {
            return;
        }
        let done = move |this: &mut Self,
                         result: Result<Option<UploadFile>, String>,
                         window: &mut Window,
                         cx: &mut Context<Self>| {
            this.image_picker = None;
            match result {
                Ok(Some(file)) => this.run(
                    window,
                    cx,
                    async move {
                        file.validate(8_000_000)?;
                        let frames = crate::shared::images::preview_frames(
                            file.bytes.clone(),
                            if background { 960 } else { 128 },
                        )
                        .await
                        .ok_or_else(|| {
                            CommandError::new(
                                stonks_core::error::ErrorCode::InvalidInput,
                                "The selected image could not be decoded.",
                            )
                        })?;
                        Ok((file, frames))
                    },
                    move |this, (file, frames), _, _| {
                        if let Some(Data::Profile(form)) = &mut this.data {
                            let selected = Some(Selected(Arc::new(file), texture(frames)));
                            if background {
                                form.next_background = selected;
                                form.remove_background = false;
                            } else {
                                form.next_avatar = selected;
                            }
                        }
                    },
                ),
                Ok(None) => {}
                Err(error) => this.error = Some(error),
            }
            cx.notify();
        };
        self.image_picker = Some(if background {
            crate::shared::file_picker::pick_image(window, cx, done)
        } else {
            crate::shared::file_picker::pick_cropped_image(window, cx, done)
        });
        cx.notify();
    }
    fn save_profile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Data::Profile(form)) = &self.data else {
            return;
        };
        if self.is_busy() || !form.dirty(cx) {
            return;
        }
        let draft = form.draft(cx);
        let avatar = form.next_avatar.as_ref().map(|s| s.0.clone());
        let background = form.next_background.as_ref().map(|s| s.0.clone());
        let service = self.service.clone();
        self.run(
            window,
            cx,
            async move {
                service
                    .save_profile_with_images(&draft, avatar.as_deref(), background.as_deref())
                    .await?;
                Ok(draft)
            },
            |this, mut draft, _, cx| {
                if let Some(Data::Profile(form)) = &mut this.data {
                    if let Some(Selected(_, preview)) = form.next_avatar.take() {
                        form.avatar = Some(preview);
                    }
                    if let Some(Selected(_, preview)) = form.next_background.take() {
                        form.background = Some(preview);
                        form.has_background = true;
                    }
                    if draft.remove_background {
                        form.background = None;
                        form.has_background = false;
                    }
                    draft.remove_background = false;
                    form.remove_background = false;
                    form.initial = draft;
                }
                this.notice = Some(t("settings.notices.profileSaved"));
                cx.emit(AccountPageEvent::ProfileSaved);
            },
        );
    }
}
