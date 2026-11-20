use super::*;
use std::collections::BTreeSet;
use stonks_core::connection::{CreateGroupRequest, create_group};

pub(super) struct Group {
    name: Entity<InputState>,
    _observe_name: Subscription,
    recipients: BTreeSet<String>,
    request: Option<Task<()>>,
    error: Option<String>,
}

fn friends(snapshot: &AppSnapshot) -> impl Iterator<Item = &ClientUser> {
    snapshot
        .users
        .iter()
        .filter(|user| user.relationship.as_deref() == Some("Friend"))
}

impl ShellView {
    pub(in crate::features) fn open_home_group(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.home_messages.group.is_some() {
            return;
        }
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Group name"));
        let observe = cx.subscribe(&name, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_home_group(cx);
            }
            cx.notify();
        });
        name.update(cx, |input, cx| input.focus(window, cx));
        self.home_messages.group = Some(Group {
            name,
            _observe_name: observe,
            recipients: Default::default(),
            request: None,
            error: None,
        });
        cx.notify();
    }

    fn submit_home_group(&mut self, cx: &mut Context<Self>) {
        let session = self.home_session().clone();
        let Some(group) = self.home_messages.group.as_mut() else {
            return;
        };
        if group.request.is_some() {
            return;
        }
        let session = session.read(cx);
        let Some((policy, context)) = session.account_context() else {
            group.error = Some("Your session expired. Log in again.".into());
            cx.notify();
            return;
        };
        let name = group.name.read(cx).value().trim().to_owned();
        let recipients: Vec<_> = session
            .snapshot
            .iter()
            .flat_map(friends)
            .filter(|friend| group.recipients.contains(&friend.id))
            .map(|friend| friend.id.clone())
            .collect();
        if !(1..=32).contains(&name.chars().count()) || !(1..=49).contains(&recipients.len()) {
            return;
        }
        let request = CreateGroupRequest { name, recipients };
        let scope = self.home_messages.scope.clone();
        let service_policy = policy.clone();
        let service_context = context.clone();
        group.error = None;
        group.request = Some(cx.spawn(async move |this, cx| {
            let result = crate::runtime::spawn_core(async move {
                if !service_policy.is_authenticated_context_current(&service_context) {
                    return None;
                }
                Some(create_group(&service_policy, request).await)
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                if this.home_messages.scope != scope {
                    return;
                }
                if let Some(group) = &mut this.home_messages.group {
                    group.request = None;
                }
                if !policy.is_authenticated_context_current(&context) {
                    cx.notify();
                    return;
                }
                match result {
                    Some(Ok(channel)) => {
                        let id = channel.id.clone();
                        this.home_messages.group = None;
                        this.home_session()
                            .update(cx, |session, cx| session.upsert_channel(channel, cx));
                        this.open_home_conversation(id, cx);
                    }
                    Some(Err(error)) => {
                        if let Some(group) = &mut this.home_messages.group {
                            group.error = Some(error.to_string());
                        }
                    }
                    None => {}
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn render_home_group(
        &self,
        group: &Group,
        snapshot: Option<&AppSnapshot>,
        bottom_inset: Pixels,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let saving = group.request.is_some();
        let name = group.name.read(cx).value();
        let name_len = name.chars().count();
        let valid_name = (1..=32).contains(&name.trim().chars().count());
        let mut candidates: Vec<_> = snapshot.into_iter().flat_map(friends).collect();
        candidates.sort_by_cached_key(|friend| user_name(friend).to_lowercase());
        let active_count = candidates
            .iter()
            .filter(|friend| group.recipients.contains(&friend.id))
            .count();
        let session = self.home_session().read(cx);
        let autumn = session
            .server
            .as_ref()
            .and_then(|server| server.endpoints.autumn.clone());
        let mut picker = div()
            .id("home-group-picker")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        if candidates.is_empty() {
            picker = picker.child(
                div()
                    .text_color(theme.text_muted)
                    .child("No friends are available to add."),
            );
        }
        for friend in candidates {
            let id = friend.id.clone();
            let selected = group.recipients.contains(&id);
            let url = avatar_url(friend, autumn.as_deref());
            let image = self
                .home_images()
                .update(cx, |images, cx| images.get(url.as_deref(), 80, cx));
            picker = picker.child(
                Button::new(
                    ElementId::Name(format!("home-group-friend-{id}").into()),
                    theme,
                )
                .variant(ButtonVariant::Channel)
                .selected(selected)
                .disabled(saving || (!selected && active_count >= 49))
                .leading(stonks_ui::components::avatar(
                    &theme,
                    user_name(friend),
                    32.,
                    image,
                ))
                .label(user_name(friend).to_owned())
                .when(selected, |button| {
                    button.trailing(Icon::Check.render(theme.accent, 18.))
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(group) = &mut this.home_messages.group {
                        if !group.recipients.remove(&id) {
                            group.recipients.insert(id.clone());
                        }
                        cx.notify();
                    }
                })),
            );
        }
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .p(px(theme.space2))
            .pb(bottom_inset + px(theme.space2))
            .gap(px(theme.space2))
            .child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .child("Create a new group"),
            )
            .when_some(group.error.as_deref(), |form, error| {
                form.child(div().text_color(theme.danger).child(error.to_owned()))
            })
            .child("Group name")
            .child(Input::new(&group.name).disabled(saving))
            .child(
                div()
                    .text_color(theme.text_muted)
                    .child(format!("{name_len}/32")),
            )
            .child("Select friends to add")
            .child(picker)
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(theme.space2))
                    .child(
                        Button::new("home-group-close", theme)
                            .variant(ButtonVariant::Secondary)
                            .label(t("common.close"))
                            .disabled(saving)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.home_messages.group = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("home-group-submit", theme)
                            .label(if saving { "Creating…" } else { "Create" })
                            .disabled(saving || !valid_name || !(1..=49).contains(&active_count))
                            .on_click(cx.listener(|this, _, _, cx| this.submit_home_group(cx))),
                    ),
            )
    }
}
