//! Timeline action eligibility, dispatch, and mutation completion.

use std::rc::Rc;

use gpui_kit::*;
use stonks_core::connection::GatewayEvent;
use stonks_core::messages::models::TimelineMessage;
use stonks_core::messages::{actions as mutations, reactions as reaction_service};
use stonks_ui::{
    components::{Button, ButtonVariant, UiContextMenu},
    i18n::t,
    icons::Icon,
    Theme,
};

use crate::runtime::spawn_core;

use super::super::reactions;
use super::super::{message_timestamp, Timeline, TimelineEvent};
use super::policy::{ActionEligibility, MenuAnchor, MessageAction, MessageActions};
use super::surface::{ActionRequest, DeletePreview, OpenAction, Submission};

enum Mutation {
    Edit(String),
    Delete,
    Pin(bool),
    Report(String),
    React(String),
}
enum MutationResult {
    Edit(Option<String>, Option<String>),
    Delete,
    Pin(bool),
    Report,
    React(String),
}

impl Timeline {
    pub fn action_menu(&self) -> Entity<UiContextMenu> {
        self.action_menu.clone()
    }

    pub(in super::super) fn action_message(&self, id: &str) -> Option<&TimelineMessage> {
        self.messages
            .binary_search_by(|message| message.id.as_str().cmp(id))
            .ok()
            .map(|index| &self.messages[index])
    }

    fn message_link(&self, message: &TimelineMessage, cx: &App) -> Option<String> {
        let session = self.session.read(cx);
        let deployment = &session.server.as_ref()?.deployment_url;
        let channel = session
            .snapshot
            .as_ref()?
            .channels
            .iter()
            .find(|channel| channel.id == message.channel_id)?;
        message_link(
            deployment,
            channel.server.as_deref(),
            &message.channel_id,
            &message.id,
        )
    }

    pub(in super::super) fn message_actions(
        &self,
        message: &TimelineMessage,
        cx: &App,
    ) -> MessageActions {
        let session = self.session.read(cx);
        let channel = &message.channel_id;
        MessageActions::new(
            ActionEligibility {
                actionable: !message.system
                    && stonks_core::messages::normalize::valid_id(&message.id)
                    && self.channel_id.as_ref() == Some(channel)
                    && session.can_view_channel(channel),
                own: session
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.current_user.id == message.author_id),
                can_send: session.can_channel_permission(channel, mutations::SEND_MESSAGE),
                can_react: session.can_channel_permission(channel, reaction_service::REACT),
                can_manage: session
                    .can_channel_permission(channel, stonks_core::channel_actions::MANAGE_MESSAGES),
                pinned: message.pinned,
                has_text: message
                    .content
                    .as_deref()
                    .is_some_and(|text| !text.is_empty()),
                has_link: self.message_link(message, cx).is_some(),
                developer_mode: cx
                    .try_global::<crate::preferences::NativePreferences>()
                    .is_some_and(|prefs| prefs.saved.developer_mode),
            },
            |action| {
                !matches!(action, MessageAction::Pin | MessageAction::Unpin)
                    || !self.pending_pins.contains(&message.id)
            },
        )
    }

    pub(in super::super) fn dispatch_action(
        &mut self,
        id: &str,
        action: MessageAction,
        anchor: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(message) = self.action_message(id) else {
            return;
        };
        if !self.message_actions(message, cx).contains(action) {
            return;
        }
        let value = match action {
            MessageAction::Reply => {
                cx.emit(TimelineEvent::Reply(Box::new(message.clone())));
                return;
            }
            MessageAction::CopyText => message.content.clone(),
            MessageAction::CopyId => Some(message.id.clone()),
            MessageAction::CopyLink => self.message_link(message, cx),
            MessageAction::MarkUnread => {
                let channel = message.channel_id.clone();
                let previous = self
                    .messages
                    .iter()
                    .rev()
                    .find(|entry| {
                        entry.id.as_str() < id
                            && stonks_core::messages::normalize::valid_id(&entry.id)
                    })
                    .map(|entry| entry.id.clone());
                self.session.update(cx, |session, cx| {
                    session.mark_unread(&channel, previous.as_deref(), cx)
                });
                return;
            }
            MessageAction::Pin | MessageAction::Unpin => {
                let channel = message.channel_id.clone();
                self.pending_pins.insert(id.to_owned());
                self.run_mutation(
                    channel,
                    id.to_owned(),
                    Mutation::Pin(action == MessageAction::Pin),
                    None,
                    window,
                    cx,
                );
                cx.notify();
                return;
            }
            _ => {
                let open = match action {
                    MessageAction::React => {
                        self.refresh_action_emojis(cx);
                        OpenAction::React(anchor)
                    }
                    MessageAction::Edit => {
                        OpenAction::Edit(message.content.clone().unwrap_or_default())
                    }
                    MessageAction::Delete => OpenAction::Delete(DeletePreview {
                        author: message.author_name.clone().into(),
                        timestamp: message_timestamp(message).into(),
                        body: message
                            .content
                            .clone()
                            .filter(|text| !text.is_empty())
                            .or_else(|| {
                                message
                                    .attachments
                                    .first()
                                    .map(|file| file.filename.clone())
                            })
                            .unwrap_or_default()
                            .into(),
                        avatar: None,
                    }),
                    MessageAction::Report => OpenAction::Report,
                    _ => return,
                };
                self.action_ui
                    .update(cx, |ui, cx| ui.open(id.to_owned(), open, window, cx));
                return;
            }
        };
        if let Some(value) = value {
            cx.write_to_clipboard(ClipboardItem::new_string(value));
        }
    }

    pub(in super::super) fn refresh_action_emojis(&self, cx: &mut Context<Self>) {
        let catalog = self.reactions.read(cx).picker_catalog(cx);
        self.action_ui
            .read(cx)
            .picker()
            .clone()
            .update(cx, |picker, cx| picker.set_custom(catalog, cx));
    }

    pub(in super::super) fn submit_action(
        &mut self,
        request: &ActionRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (action, mutation) = match &request.submission {
            Submission::React { value, custom } => {
                if *custom && !self.reactions.read(cx).can_add(value, cx) {
                    self.action_ui.update(cx, |ui, cx| {
                        ui.finish(
                            request.ticket,
                            Err("This emoji is no longer available.".into()),
                            window,
                            cx,
                        )
                    });
                    return;
                }
                (MessageAction::React, Mutation::React(value.clone()))
            }
            Submission::Edit { draft } => (MessageAction::Edit, Mutation::Edit(draft.clone())),
            Submission::Delete => (MessageAction::Delete, Mutation::Delete),
            Submission::Report { details } => {
                (MessageAction::Report, Mutation::Report(details.clone()))
            }
        };
        let id = &request.message_id;
        if let Mutation::React(emoji) = &mutation {
            let already_present = self
                .session
                .read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| {
                    self.action_message(id).is_some_and(|message| {
                        message.reactions.iter().any(|reaction| {
                            &reaction.emoji == emoji
                                && reaction.user_ids.contains(&snapshot.current_user.id)
                        })
                    })
                });
            if already_present {
                self.action_ui
                    .update(cx, |ui, cx| ui.finish(request.ticket, Ok(()), window, cx));
                return;
            }
        }
        if let Some(message) = self
            .action_message(id)
            .filter(|message| self.message_actions(message, cx).contains(action))
        {
            self.run_mutation(
                message.channel_id.clone(),
                id.clone(),
                mutation,
                Some(request.ticket),
                window,
                cx,
            );
        } else {
            self.action_ui.update(cx, |ui, cx| {
                ui.finish(
                    request.ticket,
                    Err("This action is no longer available.".into()),
                    window,
                    cx,
                )
            });
        }
    }

    fn run_mutation(
        &mut self,
        channel: String,
        id: String,
        mutation: Mutation,
        ticket: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let generation = self.generation;
        let policy = self.policy.clone();
        let auth = self.session.read(cx).authorization();
        let user = self
            .session
            .read(cx)
            .snapshot
            .as_ref()
            .map(|s| s.current_user.id.clone())
            .unwrap_or_default();
        cx.spawn_in(window, async move |this, cx| {
            let request_channel = channel.clone();
            let request_id = id.clone();
            let result = spawn_core(async move {
                let channel = &request_channel;
                let id = &request_id;
                match mutation {
                    Mutation::Edit(draft) => mutations::edit_message(&policy, &auth, channel, id, &draft).await.map(|message| MutationResult::Edit(message.content, message.edited_at)),
                    Mutation::Delete => mutations::delete_message(&policy, &auth, channel, id).await.map(|_| MutationResult::Delete),
                    Mutation::Pin(pinned) => mutations::set_message_pinned(&policy, &auth, channel, id, pinned).await.map(|_| MutationResult::Pin(pinned)),
                    Mutation::Report(details) => mutations::report_message(&policy, id, &details).await.map(|_| MutationResult::Report),
                    Mutation::React(emoji) => reaction_service::set_membership(&policy, &auth, &stonks_core::messages::models::ReactionRequest { channel_id: channel.clone(), message_id: id.clone(), emoji: emoji.clone() }, true).await.map(|_| MutationResult::React(emoji)),
                }
            }).await;
            this.update_in(cx, |this, window, cx| {
                if this.generation != generation || this.channel_id.as_ref() != Some(&channel) { return; }
                if ticket.is_none() { this.pending_pins.remove(&id); }
                let result = result.map_err(|error| error.to_string());
                if let Some(ticket) = ticket {
                    let status = result.as_ref().map(|_| ()).map_err(|error| SharedString::from(error.clone()));
                    this.action_ui.update(cx, |ui, cx| ui.finish(ticket, status, window, cx));
                }
                let (kind, data) = match result {
                    Ok(MutationResult::Delete) => ("MessageDelete", serde_json::json!({"channel": channel, "id": id})),
                    Ok(MutationResult::Pin(pinned)) => ("MessageUpdate", serde_json::json!({"channel": channel, "id": id, "data": {"pinned": pinned}})),
                    Ok(MutationResult::Edit(content, edited)) => ("MessageUpdate", serde_json::json!({"channel": channel, "id": id, "data": {"content": content, "edited": edited}})),
                    Ok(MutationResult::React(emoji)) => {
                        if let Ok(index) = this.messages.binary_search_by(|message| message.id.cmp(&id)) {
                            reactions::set_membership(&mut this.messages[index].reactions, &emoji, &user, true);
                            this.list.remeasure_items(index + 1..index + 2);
                        }
                        cx.notify();
                        return;
                    }
                    Ok(MutationResult::Report) => return,
                    Err(error) => {
                        if ticket.is_none() { this.mutation_error = Some(error); this.list.remeasure_items(0..1); }
                        cx.notify();
                        return;
                    }
                };
                this.session.update(cx, |session, cx| session.project_message_mutation(GatewayEvent { event_type: kind.into(), data }, cx));
                cx.notify();
            }).ok();
        }).detach();
    }

    pub fn render_action_overlay(&self, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        self.action_ui.update(cx, |ui, cx| ui.overlay(window, cx))
    }

    pub fn clear_action_ui(&self, window: &mut Window, cx: &mut App) {
        self.action_ui.update(cx, |ui, cx| ui.clear(window, cx));
    }

    pub fn has_action_ui(&self, cx: &App) -> bool {
        self.action_ui.read(cx).target().is_some()
    }

    pub(in super::super) fn open_message_menu(
        &mut self,
        id: &str,
        anchor: MenuAnchor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(message) = self.action_message(id) else {
            return;
        };
        if self
            .message_actions(message, cx)
            .open_menu(&self.action_menu, anchor, window, cx)
        {
            let bounds = match anchor {
                MenuAnchor::Pointer(point) => Bounds::new(point, size(px(0.), px(0.))),
                MenuAnchor::Toolbar(bounds) => bounds,
            };
            self.action_target = Some((id.to_owned(), bounds));
            cx.stop_propagation();
            cx.notify();
        }
    }

    pub(in super::super) fn add_reaction_button(
        &self,
        message: &TimelineMessage,
        theme: Theme,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        if message.reactions.is_empty()
            || !self
                .message_actions(message, cx)
                .contains(MessageAction::React)
        {
            return None;
        }
        let timeline = cx.entity().downgrade();
        let id = message.id.clone();
        let anchor = Rc::new(std::cell::Cell::new(None));
        let painted_anchor = anchor.clone();
        let button = Button::new(format!("add-reaction-{id}"), theme)
            .variant(ButtonVariant::Quiet)
            .icon_only(true)
            .expanded(self.action_ui.read(cx).reacting(&id))
            .tooltip(t("messages.addReaction"))
            .leading(Icon::Plus.render_stroke(theme.text, 18., 1.8))
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                if let Some(bounds) = anchor.get() {
                    timeline
                        .update(cx, |this, cx| {
                            this.dispatch_action(&id, MessageAction::React, bounds, window, cx)
                        })
                        .ok();
                }
            });
        Some(
            div()
                .relative()
                .child(button)
                .child(
                    canvas(
                        move |bounds, _, _| painted_anchor.set(Some(bounds)),
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .inset_0(),
                )
                .into_any_element(),
        )
    }
}

fn message_link(
    deployment: &str,
    server: Option<&str>,
    channel: &str,
    message: &str,
) -> Option<String> {
    let mut url = url::Url::parse(deployment).ok()?;
    url.set_query(None);
    url.set_fragment(None);
    {
        let mut path = url.path_segments_mut().ok()?;
        path.pop_if_empty();
        if let Some(server) = server {
            path.extend(["server", server]);
        }
        path.extend(["channel", channel, message]);
    }
    Some(url.into())
}

#[cfg(test)]
mod tests {
    use super::message_link;

    #[test]
    fn links_match_server_and_direct_channel_routes() {
        assert_eq!(
            message_link(
                "https://chat.example/base/",
                Some("server"),
                "room",
                "message"
            )
            .as_deref(),
            Some("https://chat.example/base/server/server/channel/room/message")
        );
        assert_eq!(
            message_link("https://chat.example", None, "room", "message").as_deref(),
            Some("https://chat.example/channel/room/message")
        );
    }
}
