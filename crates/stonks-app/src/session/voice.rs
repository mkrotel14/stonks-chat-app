//! Keep the Ready voice roster current independently of the local audio call.
use stonks_core::connection::{
    AppSnapshot, ClientVoiceParticipant, ClientVoiceState, GatewayEvent,
};

pub(super) fn apply(snapshot: &mut AppSnapshot, event: &GatewayEvent) {
    let data = &event.data;
    let states = &mut snapshot.voice_states;
    match event.event_type.as_str() {
        "VoiceChannelJoin" => {
            let (Some(channel), Some(participant)) = (data["id"].as_str(), decode(&data["state"]))
            else {
                return;
            };
            let user = if participant.id == snapshot.current_user.id {
                Some(&snapshot.current_user)
            } else {
                snapshot.users.iter().find(|user| user.id == participant.id)
            };
            if !user.is_some_and(|user| user.bot.as_ref().is_some_and(|bot| !bot.is_null())) {
                for state in states.iter_mut().filter(|state| state.id != channel) {
                    state
                        .participants
                        .retain(|entry| entry.id != participant.id);
                }
            }
            insert(states, channel, participant);
        }
        "VoiceChannelLeave" => {
            if let (Some(channel), Some(user)) = (data["id"].as_str(), data["user"].as_str()) {
                remove(states, channel, user);
            }
        }
        "VoiceChannelMove" => {
            if let (Some(from), Some(to), Some(user), Some(participant)) = (
                data["from"].as_str(),
                data["to"].as_str(),
                data["user"].as_str(),
                decode(&data["state"]),
            ) {
                if participant.id == user {
                    remove(states, from, user);
                    insert(states, to, participant);
                }
            }
        }
        "UserVoiceStateUpdate" => {
            let Some(channel) = data["channel_id"]
                .as_str()
                .or_else(|| data["channelId"].as_str())
            else {
                return;
            };
            let Some(user) = data["id"].as_str() else {
                return;
            };
            let Some(entry) = states
                .iter_mut()
                .find(|state| state.id == channel)
                .and_then(|state| state.participants.iter_mut().find(|entry| entry.id == user))
            else {
                return;
            };
            let patch = &data["data"];
            if let Some(value) = patch["screensharing"].as_bool() {
                entry.screensharing = value;
            }
            if let Some(value) = patch["camera"].as_bool() {
                entry.camera = value;
            }
            if let Some(value) = patch["is_receiving"]
                .as_bool()
                .or_else(|| patch["isReceiving"].as_bool())
            {
                entry.is_receiving = Some(value);
            }
            if let Some(value) = patch["is_publishing"]
                .as_bool()
                .or_else(|| patch["isPublishing"].as_bool())
            {
                entry.is_publishing = Some(value);
            }
        }
        _ => {}
    }
}

pub(super) fn apply_created_server(snapshot: &mut AppSnapshot, data: &serde_json::Value) {
    let Some(raw) = data.get("voice_states").or_else(|| data.get("voiceStates")) else {
        return;
    };
    let Ok(states) = serde_json::from_value::<Vec<ClientVoiceState>>(raw.clone()) else {
        return;
    };
    for state in states {
        super::upsert(&mut snapshot.voice_states, state, |state| state.id.clone());
    }
}

fn decode(value: &serde_json::Value) -> Option<ClientVoiceParticipant> {
    serde_json::from_value::<ClientVoiceParticipant>(value.clone())
        .ok()
        .filter(|entry| !entry.id.is_empty())
}

fn insert(states: &mut Vec<ClientVoiceState>, channel: &str, participant: ClientVoiceParticipant) {
    if let Some(state) = states.iter_mut().find(|state| state.id == channel) {
        if let Some(entry) = state
            .participants
            .iter_mut()
            .find(|entry| entry.id == participant.id)
        {
            *entry = participant;
        } else {
            state.participants.push(participant);
        }
    } else {
        states.push(ClientVoiceState {
            id: channel.to_owned(),
            participants: vec![participant],
        });
    }
}

fn remove(states: &mut Vec<ClientVoiceState>, channel: &str, user: &str) {
    if let Some(state) = states.iter_mut().find(|state| state.id == channel) {
        state.participants.retain(|entry| entry.id != user);
    }
    states.retain(|state| !state.participants.is_empty());
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn live_channel_and_server_create_keep_rail_rosters_addressable() {
        let mut session = super::super::Session::new(std::sync::Arc::new(
            stonks_core::http::HttpPolicy::default(),
        ));
        session.snapshot = Some(AppSnapshot {
            current_user: serde_json::from_value(json!({"_id":"me","username":"me"})).unwrap(),
            users: vec![],
            servers: vec![],
            channels: vec![],
            members: vec![],
            unreads: vec![],
            voice_states: vec![],
        });
        let remote = json!({"id":"remote","joined_at":"2026-08-25T12:34:56Z","is_receiving":true,"is_publishing":true,"screensharing":false,"camera":false});
        for (kind, data) in [
            (
                "ServerCreate",
                json!({"id":"s","server":{"_id":"s","name":"Server","owner":"me","channels":["existing"]},"channels":[{"_id":"existing","channel_type":"TextChannel","server":"s","name":"Existing","voice":{}}],"voice_states":[{"id":"existing","participants":[remote]}]}),
            ),
            (
                "ChannelCreate",
                json!({"_id":"new","channel_type":"TextChannel","server":"s","name":"New","voice":{}}),
            ),
            ("VoiceChannelJoin", json!({"id":"new","state":remote})),
        ] {
            session.apply_gateway_event(&GatewayEvent {
                event_type: kind.into(),
                data,
            });
            if kind == "ServerCreate" {
                assert_eq!(
                    session.snapshot.as_ref().unwrap().voice_states[0].participants[0].id,
                    "remote"
                );
            }
        }
        let snapshot = session.snapshot.as_ref().unwrap();
        assert_eq!(snapshot.servers[0].channels, ["existing", "new"]);
        assert!(snapshot
            .voice_states
            .iter()
            .any(|room| room.id == "new" && room.participants[0].id == "remote"));
    }

    #[test]
    fn peer_leave_and_rejoin_preserve_local_roster_entry() {
        let participant = |id: &str, joined_at: &str| json!({"id":id,"joined_at":joined_at});
        let mut snapshot = AppSnapshot {
            current_user: serde_json::from_value(json!({"_id":"me","username":"me"})).unwrap(),
            users: vec![],
            servers: vec![],
            channels: vec![],
            members: vec![],
            unreads: vec![],
            voice_states: vec![ClientVoiceState {
                id: "room".into(),
                participants: vec![decode(&participant("me", "initial")).unwrap()],
            }],
        };
        for (kind, data, count) in [
            (
                "VoiceChannelJoin",
                json!({"id":"room","state":participant("peer", "initial")}),
                2,
            ),
            ("VoiceChannelLeave", json!({"id":"room","user":"peer"}), 1),
            (
                "VoiceChannelJoin",
                json!({"id":"room","state":participant("peer", "rejoined")}),
                2,
            ),
        ] {
            apply(
                &mut snapshot,
                &GatewayEvent {
                    event_type: kind.into(),
                    data,
                },
            );
            let roster = &snapshot.voice_states[0].participants;
            assert_eq!(roster.len(), count);
            assert_eq!(roster[0].id, "me");
        }
        assert_eq!(
            snapshot.voice_states[0].participants[1].joined_at,
            "rejoined"
        );
    }

    #[test]
    fn follows_other_users_without_a_local_call() {
        let mut snapshot = AppSnapshot {
            current_user: serde_json::from_value(json!({"_id":"self","username":"self"})).unwrap(),
            users: vec![],
            servers: vec![],
            channels: vec![],
            members: vec![],
            unreads: vec![],
            voice_states: vec![],
        };
        let mut event = |kind: &str, data| {
            apply(
                &mut snapshot,
                &GatewayEvent {
                    event_type: kind.into(),
                    data,
                },
            )
        };
        event(
            "VoiceChannelJoin",
            json!({"id":"a","state":{"id":"remote","joined_at":"2026-08-25T12:34:56Z","is_receiving":true,"is_publishing":true,"screensharing":false,"camera":false}}),
        );
        event(
            "VoiceChannelMove",
            json!({"from":"a","to":"b","user":"remote","state":{"id":"remote","joined_at":"2026-08-25T12:34:56Z","is_receiving":true,"is_publishing":true,"screensharing":false,"camera":false}}),
        );
        event(
            "UserVoiceStateUpdate",
            json!({"channel_id":"b","id":"remote","data":{"screensharing":true,"is_publishing":false}}),
        );
        assert_eq!(snapshot.voice_states.len(), 1);
        assert_eq!(snapshot.voice_states[0].id, "b");
        let entry = &snapshot.voice_states[0].participants[0];
        assert!(entry.screensharing);
        assert_eq!(entry.is_publishing, Some(false));
        assert_eq!(entry.is_receiving, Some(true));
        assert_eq!(entry.joined_at, "2026-08-25T12:34:56Z");
        apply(
            &mut snapshot,
            &GatewayEvent {
                event_type: "VoiceChannelLeave".into(),
                data: json!({"id":"b","user":"remote"}),
            },
        );
        assert!(snapshot.voice_states.is_empty());
    }
}
