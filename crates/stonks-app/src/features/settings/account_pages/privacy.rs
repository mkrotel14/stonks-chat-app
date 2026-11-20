use super::*;
use stonks_ui::components::Switch;

impl AccountPage {
    pub(super) fn privacy_preferences(&self, theme: Theme, cx: &mut Context<Self>) -> Div {
        let Some((prefs, _)) = &self.privacy else {
            return div();
        };
        let row = |key: &'static str, checked, change: fn(bool) -> PrivacyChange| {
            let label = t(&format!("settings.privacy.{key}"));
            let entity = cx.entity().downgrade();
            settings_row(
                &theme,
                &label,
                &t(&format!("settings.privacy.{key}Description")),
                Switch::new(key, theme)
                    .label(label.clone())
                    .checked(checked)
                    .on_change(move |next, window, cx| {
                        entity
                            .update(cx, |page, cx| {
                                page.change_privacy(change(*next), window, cx)
                            })
                            .ok();
                    }),
            )
        };
        // The settings host already provides the initial section's top inset.
        let notifications = settings_section(&theme, &t("settings.privacy.notifications"))
            .mt(px(0.))
            .child(row(
                "notificationContent",
                prefs.show_notification_content,
                PrivacyChange::ShowNotificationContent,
            ))
            .child(subtitle(
                theme,
                "settings.privacy.notificationContentNativeNote",
                true,
            ));
        let messaging = settings_section(&theme, &t("settings.privacy.messaging"))
            .child(row(
                "typing",
                prefs.send_typing_indicators,
                PrivacyChange::TypingIndicators,
            ))
            .child(row(
                "linkWarnings",
                prefs.warn_external_links,
                PrivacyChange::LinkWarnings,
            ))
            .child(row(
                "embeds",
                prefs.display_embeds,
                PrivacyChange::DisplayEmbeds,
            ))
            .child(
                div().flex().mt(px(theme.space1)).child(
                    Button::new("clear-trusted-domains", theme)
                        .variant(ButtonVariant::Secondary)
                        .label(t("settings.privacy.clearTrusted"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.change_privacy(PrivacyChange::ClearTrustedDomains, window, cx)
                        })),
                ),
            );
        div().child(notifications).child(messaging)
    }

    fn change_privacy(
        &mut self,
        change: PrivacyChange,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((prefs, write)) = &mut self.privacy else {
            return;
        };
        self.notice = None;
        self.error = write(change, window, cx).err();
        if self.error.is_none() {
            match change {
                PrivacyChange::ShowNotificationContent(value) => {
                    prefs.show_notification_content = value;
                }
                PrivacyChange::TypingIndicators(value) => prefs.send_typing_indicators = value,
                PrivacyChange::DisplayEmbeds(value) => prefs.display_embeds = value,
                PrivacyChange::LinkWarnings(value) => prefs.warn_external_links = value,
                PrivacyChange::ClearTrustedDomains => {
                    self.notice = Some(t("settings.privacy.trustedCleared"));
                }
            }
        }
        cx.notify();
    }
    pub(super) fn blocked_users(
        &self,
        users: &[ClientUser],
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut section = settings_section(&theme, &t("settings.privacy.blockedUsers")).child(
            subtitle(theme, "settings.privacy.blockedUsersDescription", false)
                .max_w(px(630.))
                .mt(px(theme.space1))
                .mb(px(theme.space5))
                .line_height(px(theme.font_size_sm * theme.line_height_normal)),
        );
        if users.is_empty() {
            return section.child(
                div()
                    .flex()
                    .flex_col()
                    .min_h(px(260.))
                    .items_center()
                    .justify_center()
                    .text_center()
                    .text_color(theme.text)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(62.))
                            .rounded_full()
                            .bg(theme.surface_selected)
                            .mb(px(theme.space4))
                            .text_size(px(27.))
                            .child("✓"),
                    )
                    .child(
                        div()
                            .mb(px(theme.space1))
                            .text_size(px(theme.font_size_lg))
                            .font_weight(FontWeight::BOLD)
                            .child(t("settings.privacy.noBlockedUsers")),
                    ),
            );
        }
        for user in users {
            let id = user.id.clone();
            section = section.child(settings_row(
                &theme,
                user.display_name.as_deref().unwrap_or(&user.username),
                &user.username,
                Button::new(SharedString::from(format!("unblock-{id}")), theme)
                    .variant(ButtonVariant::Secondary)
                    .label(t("settings.privacy.unblock"))
                    .disabled(self.pending.is_some())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let id = id.clone();
                        let service = this.service.clone();
                        this.run(
                            window,
                            cx,
                            async move { service.unblock(&id).await },
                            |this, user, _, cx| {
                                if let Some(Data::Privacy(users)) = &mut this.data {
                                    let name = users
                                        .iter()
                                        .find(|item| item.id == user.id)
                                        .map(|item| {
                                            item.display_name.as_deref().unwrap_or(&item.username)
                                        })
                                        .unwrap_or(&user.username);
                                    this.notice = Some(t_vars(
                                        "settings.privacy.userUnblocked",
                                        &[("name", name)],
                                    ));
                                    users.retain(|item| item.id != user.id);
                                }
                                cx.emit(AccountPageEvent::UserUnblocked(user));
                            },
                        );
                    })),
            ));
        }
        section
    }
}
