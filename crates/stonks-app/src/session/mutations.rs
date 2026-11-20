//! Confirmed mutation projection. HTTP success updates the visible snapshot;
//! gateway state remains authoritative for the connection's permission manager.
use super::{members, Session};
use gpui_kit::Context;
use serde_json::json;
use stonks_core::{
    connection::{
        AppSnapshot, ClientChannel, ClientMember, ClientMemberId, ClientUser, GatewayEvent,
    },
    member_actions::MemberPatch,
};

fn user_event(user: &ClientUser) -> GatewayEvent {
    let file = |file: &stonks_core::connection::ClientFileReference| json!({"_id":file.id,"tag":file.tag,"content_type":file.content_type});
    GatewayEvent {
        event_type: "UserUpdate".into(),
        data: json!({"id":user.id,"data":{
            "username":user.username,"discriminator":user.discriminator,"display_name":user.display_name,"pronouns":user.pronouns,
            "avatar":user.avatar.as_ref().map(file),"status":user.status,"online":user.online,"privileged":user.privileged,"relationship":user.relationship,"bot":user.bot
        }}),
    }
}
fn member_event(server: &str, user: &str, patch: &MemberPatch) -> Option<GatewayEvent> {
    let mut data = patch.body().ok()?;
    let object = data.as_object_mut()?;
    let clear = object.remove("remove").unwrap_or_else(|| json!([]));
    if let Some(id) = object
        .get("avatar")
        .and_then(|value| value.as_str())
        .map(str::to_owned)
    {
        object.insert("avatar".into(), json!({"_id":id,"tag":"avatars"}));
    }
    Some(GatewayEvent {
        event_type: "ServerMemberUpdate".into(),
        data: json!({"id":{"server":server,"user":user},"data":data,"clear":clear}),
    })
}
fn project_user(snapshot: &mut AppSnapshot, user: ClientUser) {
    if snapshot.current_user.id == user.id {
        snapshot.current_user = user.clone();
    }
    super::upsert(&mut snapshot.users, user, |user| user.id.clone());
    members::deduplicate(snapshot);
}
fn project_member(snapshot: &mut AppSnapshot, server: &str, user: &str, patch: &MemberPatch) {
    if let Some(member) = snapshot
        .members
        .iter_mut()
        .find(|member| member.id.server == server && member.id.user == user)
    {
        patch.apply(member);
    } else {
        let mut member = ClientMember {
            id: ClientMemberId {
                server: server.into(),
                user: user.into(),
            },
            nickname: None,
            pronouns: None,
            avatar: None,
            roles: vec![],
            timeout: None,
            joined_at: None,
        };
        patch.apply(&mut member);
        snapshot.members.push(member);
    }
    members::deduplicate(snapshot);
}
fn project_channel(snapshot: &mut AppSnapshot, channel: ClientChannel) {
    if let Some(server) = channel
        .server
        .as_ref()
        .and_then(|id| snapshot.servers.iter_mut().find(|server| &server.id == id))
    {
        if !server.channels.contains(&channel.id) {
            server.channels.push(channel.id.clone());
        }
    }
    super::upsert(&mut snapshot.channels, channel, |channel| {
        channel.id.clone()
    });
}
fn remove_channel(snapshot: &mut AppSnapshot, id: &str) {
    snapshot.channels.retain(|channel| channel.id != id);
    snapshot.unreads.retain(|unread| unread.id.channel != id);
    snapshot.voice_states.retain(|state| state.id != id);
    for server in &mut snapshot.servers {
        server.channels.retain(|channel| channel != id);
        for category in &mut server.categories {
            category.channels.retain(|channel| channel != id);
        }
    }
}
fn reset_channel(snapshot: &mut AppSnapshot, id: &str) {
    if let Some(channel) = snapshot
        .channels
        .iter_mut()
        .find(|channel| channel.id == id)
    {
        channel.last_message_id = None;
    }
    snapshot.unreads.retain(|unread| unread.id.channel != id);
}
impl Session {
    pub fn refresh_account_user(
        &mut self,
        service: stonks_core::account_settings::AccountSettingsService,
        cx: &mut Context<Self>,
    ) {
        use futures_util::future::{AbortHandle, Abortable};
        let (abort, registration) = AbortHandle::new_pair();
        let request = service.clone();
        let task = cx.spawn(async move |this, cx| {
            let result = crate::runtime::spawn_core(Abortable::new(
                async move { request.me().await },
                registration,
            ))
            .await;
            if let Ok(result) = result {
                let _ = this.update(cx, |this, cx| {
                    this.account_refresh = None;
                    if !service.is_current() || this.account_context().is_none() {
                        return;
                    }
                    match result {
                        Ok(user)
                            if this
                                .snapshot
                                .as_ref()
                                .is_some_and(|s| s.current_user.id == user.id) =>
                        {
                            this.update_user(user, cx)
                        }
                        Err(error) => eprintln!("session: could not refresh current user: {error}"),
                        _ => {}
                    }
                });
            }
        });
        self.account_refresh = Some(super::ReadRequest { abort, _task: task });
    }

    pub fn update_user(&mut self, user: ClientUser, cx: &mut Context<Self>) {
        if self.member_loading {
            self.member_events.push(user_event(&user));
        }
        if let Some(snapshot) = &mut self.snapshot {
            project_user(snapshot, user);
            cx.notify();
        }
    }
    pub fn update_member(
        &mut self,
        server: &str,
        user: &str,
        patch: MemberPatch,
        cx: &mut Context<Self>,
    ) {
        if self.member_loading {
            if let Some(event) = member_event(server, user, &patch) {
                self.member_events.push(event);
            }
        }
        if let Some(snapshot) = &mut self.snapshot {
            project_member(snapshot, server, user, &patch);
            cx.notify();
        }
    }
    pub fn remove_member(&mut self, server: &str, user: &str, cx: &mut Context<Self>) {
        if self.member_loading {
            self.member_events.push(GatewayEvent {
                event_type: "ServerMemberLeave".into(),
                data: json!({"id":server,"user":user}),
            });
        }
        if let Some(snapshot) = &mut self.snapshot {
            snapshot
                .members
                .retain(|member| member.id.server != server || member.id.user != user);
            cx.notify();
        }
    }
    pub fn upsert_channel(&mut self, channel: ClientChannel, cx: &mut Context<Self>) {
        if let Some(snapshot) = &mut self.snapshot {
            project_channel(snapshot, channel);
            cx.notify();
        }
    }
    pub fn remove_channel(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(snapshot) = &mut self.snapshot {
            remove_channel(snapshot, id);
        }
        self.read_state.reset();
        self.read_request = None;
        self.read_error = None;
        cx.notify();
    }
    pub fn reset_channel(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(snapshot) = &mut self.snapshot {
            reset_channel(snapshot, id);
        }
        self.read_state.reset();
        self.read_request = None;
        self.read_error = None;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> AppSnapshot {
        let user=serde_json::from_value(json!({"_id":"self","username":"Self","avatar":{"_id":"old-avatar","tag":"avatars"},"pronouns":"they/them"})).unwrap();
        AppSnapshot{current_user:user,users:vec![],servers:vec![serde_json::from_value(json!({"_id":"server","name":"Server","owner":"self","channels":["channel","other"],"categories":[{"id":"category","title":"Text","channels":["channel","other"]}]})).unwrap()],channels:vec![serde_json::from_value(json!({"_id":"channel","channel_type":"TextChannel","server":"server","last_message_id":"latest"})).unwrap()],members:vec![serde_json::from_value(json!({"_id":{"server":"server","user":"self"},"nickname":"Old","pronouns":"they/them","roles":["role"],"avatar":{"_id":"old","tag":"avatars"}})).unwrap()],unreads:vec![serde_json::from_value(json!({"_id":{"channel":"channel","user":"self"},"last_id":"old","mentions":["latest"]})).unwrap()],voice_states:vec![serde_json::from_value(json!({"id":"channel","participants":[]})).unwrap()]}
    }
    #[test]
    fn member_clear_and_user_updates_survive_stale_roster_replay() {
        let mut live = snapshot();
        let mut stale = live.clone();
        let patch = MemberPatch {
            nickname: Some(" ".into()),
            pronouns: Some(None),
            avatar: Some(Some("new-file_id".into())),
            roles: Some(vec!["newRole".into()]),
        };
        project_member(&mut live, "server", "self", &patch);
        members::apply(&mut stale, &member_event("server", "self", &patch).unwrap());
        for snapshot in [&live, &stale] {
            let member = &snapshot.members[0];
            assert_eq!(member.nickname, None);
            assert_eq!(member.pronouns, None);
            assert_eq!(member.avatar.as_ref().unwrap().id, "new-file_id");
            assert_eq!(member.roles, ["newRole"]);
        }
        let mut changed = live.current_user.clone();
        changed.pronouns = None;
        changed.avatar = None;
        changed.display_name = Some("Updated".into());
        project_user(&mut live, changed.clone());
        members::apply(&mut stale, &user_event(&changed));
        assert_eq!(
            live.current_user.display_name,
            stale.current_user.display_name
        );
        assert!(stale.current_user.avatar.is_none());
        assert!(stale.current_user.pronouns.is_none());
        assert_eq!(live.users.len(), 1);
    }
    #[test]
    fn channel_reset_keeps_membership_but_removal_cleans_all_references() {
        let mut state = snapshot();
        reset_channel(&mut state, "channel");
        assert!(state.channels[0].last_message_id.is_none());
        assert!(state.unreads.is_empty());
        assert_eq!(state.servers[0].channels.len(), 2);
        remove_channel(&mut state, "channel");
        assert!(state.channels.is_empty());
        assert!(state.voice_states.is_empty());
        assert_eq!(state.servers[0].channels, ["other"]);
        assert_eq!(state.servers[0].categories[0].channels, ["other"]);
    }
    #[test]
    fn dm_upsert_never_duplicates_a_channel_or_changes_server_channel_membership() {
        let mut state = snapshot();
        let channel: ClientChannel = serde_json::from_value(
            json!({"_id":"dm","channel_type":"DirectMessage","recipients":["self","other"]}),
        )
        .unwrap();
        project_channel(&mut state, channel.clone());
        project_channel(&mut state, channel);
        assert_eq!(state.channels.len(), 2);
        assert_eq!(state.servers[0].channels, ["channel", "other"]);
    }
}
