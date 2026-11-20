use super::extra::Dialog;
use super::*;
enum Resource {
    Emojis(Vec<ServerEmoji>),
    Invites(Vec<ServerInvite>),
}

impl ServerSettings {
    pub(super) fn load(&mut self, cx: &mut Context<Self>) {
        if self.loading
            || !matches!(self.page, Page::Emojis | Page::Invites)
            || (self.page == Page::Invites && !self.can_manage)
        {
            return;
        }
        self.list_request = None;
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let page = self.page;
        self.loading = true;
        self.list_error = None;
        let id = self.server.id.clone();
        let policy = self.policy.clone();
        let (abort, registration) = AbortHandle::new_pair();
        let task = cx.spawn(async move |this, cx| {
            let Ok(result) = spawn_core(Abortable::new(
                async move {
                    match page {
                        Page::Emojis => service::fetch_emojis(&policy, &id)
                            .await
                            .map(Resource::Emojis),
                        _ => service::fetch_invites(&policy, &id)
                            .await
                            .map(Resource::Invites),
                    }
                },
                registration,
            ))
            .await
            else {
                return;
            };
            this.update(cx, |this, cx| {
                if generation != this.generation || page != this.page {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(Resource::Emojis(emojis)) => this.emojis = Some(emojis),
                    Ok(Resource::Invites(invites)) => this.invites = Some(invites),
                    Err(error) => this.list_error = Some(error.to_string()),
                }
                cx.notify();
            })
            .ok();
        });
        self.list_request = Some(Request { abort, _task: task });
        cx.notify();
    }
    pub(super) fn resource_status(
        &self,
        mut rows: Div,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let (loaded, retry, loading) = match self.page {
            Page::Emojis => (
                self.emojis.is_some(),
                "Retry loading emojis",
                "Loading emojis…",
            ),
            Page::Invites => (
                self.invites.is_some(),
                "Retry loading invites",
                "Loading invites…",
            ),
            _ => return rows,
        };
        if let Some(error) = &self.list_error {
            rows = rows
                .child(Feedback::new(theme, FeedbackTone::Error, error.clone()))
                .child(
                    Button::new("server-settings-retry", theme)
                        .variant(ButtonVariant::Secondary)
                        .disabled(self.loading || self.busy)
                        .label(retry)
                        .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
                );
        }
        if self.loading && !loaded {
            rows = rows.child(
                div()
                    .min_h(px(180.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.text_muted)
                    .child(loading),
            );
        }
        rows
    }
    pub(super) fn resource_empty(theme: &Theme, title: &'static str) -> Div {
        div()
            .p(px(theme.space6))
            .text_center()
            .text_size(px(theme.font_size_lg))
            .font_weight(FontWeight::BOLD)
            .text_color(theme.text)
            .child(title)
    }

    pub(super) fn extra_sources(&self) -> Vec<(String, u32)> {
        let mut sources = Vec::new();
        for role in self.server.roles.values() {
            if let Some(url) = role
                .icon
                .as_ref()
                .and_then(|file| media::file_url(self.autumn.as_deref(), &file.tag, &file.id))
            {
                sources.push((url, 72));
            }
        }
        for draft in self.extra.roles.role_drafts.values() {
            if let Some(url) = &draft.icon_url {
                sources.push((url.clone(), 72));
            }
        }
        if self.page == Page::Emojis {
            if let Some(emojis) = &self.emojis {
                for emoji in emojis {
                    if let Some(url) = media::file_url(self.autumn.as_deref(), "emojis", &emoji.id)
                    {
                        sources.push((url, 34));
                    }
                }
            }
        }
        if let Some(Dialog::Emoji(emoji)) = &self.extra.dialog {
            if let Some(url) = media::file_url(self.autumn.as_deref(), "emojis", &emoji.id) {
                sources.push((url, 180));
            }
        }
        sources
    }
}
