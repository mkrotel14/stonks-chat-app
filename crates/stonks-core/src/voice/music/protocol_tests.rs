use super::*;

fn item() -> MusicQueueItem {
    MusicQueueItem {
        id: "alice:track".into(),
        title: "Track".into(),
        source_url: "https://example.com/".into(),
        artwork_url: None,
        requested_by: Some("alice".into()),
        playback_source: String::new(),
        local_file: false,
    }
}

#[test]
fn music_packets_are_versionable_json() {
    let packet = MusicPacket::Control {
        session_id: "session".into(),
        action: "pause".into(),
        item_id: None,
    };
    let value = serde_json::to_value(packet).unwrap();
    assert_eq!(value["type"], "control");
    assert_eq!(value["action"], "pause");
    assert_eq!(value["sessionId"], "session");
}

#[test]
fn serializing_music_state_removes_source_secrets_and_tool_diagnostics() {
    let secret = "path-token-7348";
    let state = MusicSnapshot {
        source_url: format!(
            "https://user:password@example.com/{secret}?token=query-secret#fragment"
        ),
        artwork_url: Some(format!(
            "https://images.example.com/cover.jpg?token={secret}"
        )),
        error: format!("yt-dlp failed at /home/alice/{secret}: bearer query-secret"),
        queue: vec![MusicQueueItem {
            source_url: format!("https://example.com/{secret}?sig=query-secret"),
            ..item()
        }],
        ..Default::default()
    };
    let encoded = serde_json::to_string(&MusicPacket::State { state }).unwrap();
    assert!(!encoded.contains(secret));
    assert!(!encoded.contains("query-secret"));
    assert!(!encoded.contains("password"));
    assert!(!encoded.contains("yt-dlp"));
    assert!(encoded.contains(PUBLIC_PLAYBACK_ERROR));
}

#[test]
fn inbound_packets_reject_unsafe_or_unbounded_public_data() {
    let mut unsafe_item = item();
    unsafe_item.artwork_url = Some("http://127.0.0.1/private.png".into());
    assert!(!MusicPacket::Add {
        session_id: "session".into(),
        item: unsafe_item,
    }
    .is_valid_inbound());

    let mut oversized = item();
    oversized.title = "x".repeat(MAX_TITLE_BYTES + 1);
    assert!(!MusicPacket::Add {
        session_id: "session".into(),
        item: oversized,
    }
    .is_valid_inbound());
}

#[test]
fn queue_validation_enforces_capacity_deduplication_and_current_item_separation() {
    let queue = (0..MAX_QUEUE_ITEMS)
        .map(|index| MusicQueueItem {
            id: format!("alice:{index}"),
            ..item()
        })
        .collect::<Vec<_>>();
    let state = MusicSnapshot {
        queue: queue.clone(),
        ..Default::default()
    };
    assert!(state.is_valid_public());

    let mut oversized = state.clone();
    oversized.queue.push(MusicQueueItem {
        id: "alice:overflow".into(),
        ..item()
    });
    assert!(!oversized.is_valid_public());

    let mut duplicate = state.clone();
    duplicate.queue[1].id = duplicate.queue[0].id.clone();
    assert!(!duplicate.is_valid_public());

    let mut repeats_current = state;
    repeats_current.current_item_id = Some(repeats_current.queue[0].id.clone());
    assert!(!repeats_current.is_valid_public());
}

#[test]
fn maximum_item_count_state_and_handoff_fit_the_packet_limit() {
    let queue = (0..MAX_QUEUE_ITEMS)
        .map(|index| MusicQueueItem {
            id: format!("alice:{index}"),
            ..item()
        })
        .collect::<Vec<_>>();
    let current = MusicQueueItem {
        id: "alice:current".into(),
        ..item()
    };
    let state = MusicSnapshot {
        session_id: "session".into(),
        revision: 1,
        coordinator_id: Some("alice".into()),
        active: true,
        host_id: Some("alice".into()),
        current_item_id: Some(current.id.clone()),
        title: current.title.clone(),
        source_url: current.source_url.clone(),
        requested_by: current.requested_by.clone(),
        queue,
        ..Default::default()
    };
    for packet in [
        MusicPacket::State {
            state: state.clone(),
        },
        MusicPacket::Handoff {
            state,
            item: current,
        },
    ] {
        assert!(packet.is_valid_inbound());
        assert!(serde_json::to_vec(&packet).unwrap().len() <= MAX_MUSIC_PACKET_BYTES);
    }
}

#[test]
fn serialized_queue_budget_rejects_pathological_metadata_before_packet_growth() {
    let mut queue = Vec::new();
    for index in 0..MAX_QUEUE_ITEMS {
        queue.push(MusicQueueItem {
            id: format!("alice:{index}"),
            title: "\"".repeat(MAX_TITLE_BYTES),
            ..item()
        });
        if !queue_is_bounded(&queue, None) {
            assert!(queue.len() < MAX_QUEUE_ITEMS);
            return;
        }
    }
    panic!("pathological metadata should exhaust the serialized queue budget");
}

#[test]
fn coordinator_identity_survives_playback_host_migration() {
    let state = MusicSnapshot {
        coordinator_id: Some("alice".into()),
        host_id: Some("bob".into()),
        ..Default::default()
    };
    assert_eq!(session_priority(&state), "alice");
}

#[test]
fn legacy_music_snapshots_default_to_an_empty_queue() {
    let state: MusicSnapshot = serde_json::from_value(serde_json::json!({
        "active": false,
        "hostId": null,
        "title": "",
        "sourceUrl": "",
        "artworkUrl": null,
        "requestedBy": null,
        "paused": false,
        "loading": false,
        "elapsedSeconds": 0,
        "durationSeconds": null,
        "error": ""
    }))
    .unwrap();
    assert!(state.queue.is_empty());
    assert_eq!(state.playback_epoch, 0);
    assert!(!state.seeking);
}

#[test]
fn every_room_packet_retains_its_tag_and_camel_case_fields() {
    let item = item();
    let state = MusicSnapshot {
        session_id: "session".into(),
        revision: 12,
        coordinator_id: Some("alice".into()),
        queue: vec![item.clone()],
        ..Default::default()
    };
    let handoff_state = MusicSnapshot {
        active: true,
        host_id: Some("alice".into()),
        current_item_id: Some(item.id.clone()),
        queue: Vec::new(),
        ..state.clone()
    };
    let packets = [
        (
            "state",
            MusicPacket::State {
                state: state.clone(),
            },
        ),
        (
            "add",
            MusicPacket::Add {
                session_id: "session".into(),
                item: item.clone(),
            },
        ),
        (
            "add_result",
            MusicPacket::AddResult {
                session_id: "session".into(),
                item_id: item.id.clone(),
                accepted: true,
            },
        ),
        (
            "metadata",
            MusicPacket::Metadata {
                session_id: "session".into(),
                item_id: item.id.clone(),
                requester: "alice".into(),
                title: "Resolved track".into(),
                source_url: item.source_url.clone(),
                artwork_url: None,
            },
        ),
        (
            "control",
            MusicPacket::Control {
                session_id: "session".into(),
                action: "remove".into(),
                item_id: Some(item.id.clone()),
            },
        ),
        (
            "seek",
            MusicPacket::Seek {
                session_id: "session".into(),
                item_id: item.id.clone(),
                observed_revision: 12,
                observed_playback_epoch: 2,
                request_id: "request-1".into(),
                target_seconds: 30,
            },
        ),
        (
            "seek_result",
            MusicPacket::SeekResult {
                session_id: "session".into(),
                item_id: item.id.clone(),
                request_id: "request-1".into(),
                accepted: true,
                revision: 13,
                playback_epoch: 3,
                elapsed_seconds: 30,
            },
        ),
        (
            "handoff",
            MusicPacket::Handoff {
                state: handoff_state,
                item,
            },
        ),
        ("sync_request", MusicPacket::SyncRequest),
    ];
    for (tag, packet) in packets {
        let encoded = serde_json::to_value(packet).unwrap();
        assert_eq!(encoded["type"], tag);
        let decoded: MusicPacket = serde_json::from_value(encoded.clone()).unwrap();
        assert_eq!(serde_json::to_value(&decoded).unwrap(), encoded);
        assert!(decoded.is_valid_inbound());
        assert!(encoded.get("session_id").is_none());
    }
}

#[test]
fn seek_packets_are_bounded_and_require_complete_identity() {
    assert!(MusicPacket::Seek {
        session_id: "session".into(),
        item_id: "alice:track".into(),
        observed_revision: 4,
        observed_playback_epoch: 2,
        request_id: "request".into(),
        target_seconds: 30,
    }
    .is_valid_inbound());
    assert!(!MusicPacket::Seek {
        session_id: "session".into(),
        item_id: "alice:track".into(),
        observed_revision: 4,
        observed_playback_epoch: 2,
        request_id: String::new(),
        target_seconds: 30,
    }
    .is_valid_inbound());
    assert!(!MusicPacket::SeekResult {
        session_id: "session".into(),
        item_id: "alice:track".into(),
        request_id: "request".into(),
        accepted: false,
        revision: 4,
        playback_epoch: 2,
        elapsed_seconds: MAX_PLAYBACK_SECONDS + 1,
    }
    .is_valid_inbound());
}

#[test]
fn remote_packets_cannot_populate_private_local_playback_fields() {
    let item: MusicQueueItem = serde_json::from_value(serde_json::json!({
        "id": "alice:track",
        "title": "Local file",
        "sourceUrl": "Local file",
        "requestedBy": "alice",
        "playbackSource": "/private/music/track.ogg",
        "localFile": true
    }))
    .unwrap();
    assert!(item.playback_source.is_empty());
    assert!(!item.local_file);
}

#[test]
fn legacy_host_priority_remains_stable_without_a_coordinator() {
    let state = MusicSnapshot {
        host_id: Some("bob".into()),
        ..Default::default()
    };
    assert_eq!(session_priority(&state), "bob");
    assert!(session_priority(&state) < session_priority(&MusicSnapshot::default()));
}
