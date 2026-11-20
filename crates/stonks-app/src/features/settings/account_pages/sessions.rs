use super::*;
use stonks_ui::icons::Icon;

// Keep the substring matching and priority in sync with sessionPresentation.ts.
fn session_device_icon(name: &str) -> Icon {
    let name = name.to_lowercase();
    let contains_any = |terms: &[&str]| terms.iter().any(|term| name.contains(*term));
    if contains_any(&["android", "iphone", "ipad", "mobile", "phone", "tablet"]) {
        Icon::SessionMobile
    } else if contains_any(&[
        "chrome", "chromium", "edge", "firefox", "safari", "browser", "web",
    ]) {
        Icon::SessionBrowser
    } else if contains_any(&[
        "desktop",
        "windows",
        "linux",
        "macos",
        "mac os",
        "tauri",
        "stonks chat",
    ]) {
        Icon::SessionDesktop
    } else {
        Icon::SessionGeneric
    }
}

fn session_row(
    theme: &Theme,
    name: &str,
    title: String,
    description: String,
    trailing: Div,
) -> Div {
    div()
        .flex()
        .flex_wrap()
        .min_w_0()
        .min_h(px(62.))
        .flex_shrink_0()
        .items_center()
        .justify_between()
        .gap(px(theme.space4))
        .py(px(theme.space3))
        .px(px(15.))
        .border_1()
        .border_color(theme.outline)
        .rounded(px(theme.radius_md))
        .bg(theme.surface)
        .child(
            div()
                .size(px(36.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(theme.radius_md))
                .bg(theme.surface_interactive)
                .child(session_device_icon(name).render_stroke(theme.accent, 21., 1.8)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .flex_grow(1.)
                .flex_basis(px(160.))
                .gap(px(theme.space1))
                .child(
                    div()
                        .text_size(px(theme.font_size_md))
                        .line_height(px(theme.font_size_md * theme.line_height_compact))
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.text)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(px(theme.font_size_xs))
                        .line_height(px(theme.font_size_xs * theme.line_height_normal))
                        .text_color(theme.text_muted)
                        .child(description),
                ),
        )
        .child(trailing.ml_auto())
}

impl AccountPage {
    pub(super) fn sessions(&self, rows: &[Session], theme: Theme, cx: &mut Context<Self>) -> Div {
        let disabled = self.pending.is_some() || self.editor.is_some();
        let known = rows.iter().any(|row| row.current == Some(true));
        let mut list = div()
            .flex()
            .flex_col()
            .gap(px(theme.space3))
            .child(subtitle(theme, "settings.sessions.description", false));
        if !known && !rows.is_empty() {
            list = list.child(subtitle(theme, "settings.sessions.currentUnknown", true));
        }
        for row in rows
            .iter()
            .filter(|r| r.current == Some(true))
            .chain(rows.iter().filter(|r| r.current != Some(true)))
        {
            let mut description = t_vars("settings.sessions.session", &[("id", &row.id)]);
            if let Some(last_seen) = &row.last_seen {
                let time = stonks_ui::i18n::format_iso_datetime(last_seen);
                if !time.is_empty() {
                    description.push_str(&format!(
                        " · {}",
                        t_vars("settings.sessions.lastSeen", &[("time", time)])
                    ));
                }
            }
            let title = if row.current == Some(true) {
                format!("{} · {}", row.name, t("settings.sessions.current"))
            } else {
                row.name.clone()
            };
            let rename = row.clone();
            let revoke = row.clone();
            list = list.child(session_row(
                &theme,
                &row.name,
                title,
                description,
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(theme.space2))
                    .child(
                        Button::new(
                            SharedString::from(format!("session-rename-{}", row.id)),
                            theme,
                        )
                        .variant(ButtonVariant::Quiet)
                        .label(t("settings.sessions.rename"))
                        .disabled(disabled)
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.open_editor(Action::Rename(rename.clone()), &[], window, cx)
                            },
                        )),
                    )
                    .child(
                        Button::new(
                            SharedString::from(format!("session-revoke-{}", row.id)),
                            theme,
                        )
                        .variant(ButtonVariant::DangerQuiet)
                        .label(t("settings.sessions.revoke"))
                        .disabled(disabled || row.current != Some(false))
                        .tooltip(t(match row.current {
                            Some(true) => "settings.sessions.currentRevokeDisabled",
                            None => "settings.sessions.unknownRevokeDisabled",
                            _ => "settings.sessions.revoke",
                        }))
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.open_editor(Action::Revoke(revoke.clone()), &[], window, cx)
                            },
                        )),
                    ),
            ));
        }
        let has_other_sessions = rows.iter().any(|row| row.current != Some(true));
        list = list.child(
            div()
                .flex()
                .justify_end()
                // Match the web list gap plus its bulk-action margin.
                .mt(px(2. * theme.space2 - theme.space3))
                .child(
                    Button::new("revoke-other-sessions", theme)
                        .variant(ButtonVariant::DangerQuiet)
                        .label(t("settings.sessions.revokeOthers"))
                        .disabled(disabled || !has_other_sessions)
                        .on_click(cx.listener(|this, _, window, cx| {
                            let service = this.service.clone();
                            this.run(
                                window,
                                cx,
                                async move { service.account().await },
                                |this, account, window, cx| {
                                    this.open_editor(
                                        Action::RevokeOthers,
                                        account.methods(),
                                        window,
                                        cx,
                                    )
                                },
                            );
                        })),
                ),
        );
        list
    }
}

#[cfg(test)]
mod tests {
    use super::{session_device_icon, Icon};

    #[test]
    fn session_device_icon_matches_tauri_keywords_and_precedence() {
        for (icon, names) in [
            (
                Icon::SessionMobile,
                &[
                    "Android",
                    "iPhone",
                    "IPAD",
                    "Mobile",
                    "Phone",
                    "Tablet",
                    "Safari on iPhone",
                    "Android Linux Web",
                ][..],
            ),
            (
                Icon::SessionBrowser,
                &[
                    "Chrome",
                    "Chromium",
                    "Edge",
                    "Firefox",
                    "Safari",
                    "Browser",
                    "Web",
                    "Firefox on Linux",
                ][..],
            ),
            (
                Icon::SessionDesktop,
                &[
                    "Desktop",
                    "Windows",
                    "Linux",
                    "macOS",
                    "Mac OS",
                    "Tauri",
                    "Stonks Chat",
                ][..],
            ),
            (Icon::SessionGeneric, &["", "Custom session", "MacBook"][..]),
        ] {
            for name in names {
                assert_eq!(session_device_icon(name), icon, "session name: {name:?}");
            }
        }
    }
}
