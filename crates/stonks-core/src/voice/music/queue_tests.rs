use super::*;

#[test]
fn participant_departure_removes_every_song_they_own() {
    let mut queue = VecDeque::from([
        queue_item("https://example.com/a".into(), "alice".into(), false),
        queue_item("https://example.com/b".into(), "bob".into(), false),
        queue_item("https://example.com/c".into(), "bob".into(), false),
    ]);
    remove_owner_items(&mut queue, "bob");
    assert_eq!(queue.len(), 1);
    assert_eq!(item_owner(queue.front().unwrap()), Some("alice"));
}

fn test_item(id: usize, owner: &str) -> MusicQueueItem {
    MusicQueueItem {
        id: format!("{owner}:{id}"),
        title: format!("Track {id}"),
        source_url: "https://example.com/".into(),
        artwork_url: None,
        requested_by: Some(owner.into()),
        playback_source: String::new(),
        local_file: false,
    }
}

#[test]
fn queue_capacity_is_exact_and_duplicates_remain_idempotent() {
    let mut queue = VecDeque::new();
    for id in 0..super::super::protocol::MAX_QUEUE_ITEMS {
        assert_eq!(
            insert_item(&mut queue, Some("playing"), test_item(id, "alice")),
            Ok(QueueInsert::Added)
        );
    }
    assert_eq!(queue.len(), super::super::protocol::MAX_QUEUE_ITEMS);
    assert_eq!(
        insert_item(&mut queue, Some("playing"), test_item(0, "alice")),
        Ok(QueueInsert::Duplicate)
    );
    assert_eq!(
        insert_item(&mut queue, Some("playing"), test_item(101, "bob")),
        Err(QueueFull)
    );
    assert_eq!(
        insert_item(&mut queue, Some("alice:101"), test_item(101, "alice")),
        Ok(QueueInsert::Duplicate)
    );
    assert_eq!(queue.len(), super::super::protocol::MAX_QUEUE_ITEMS);
}

#[test]
fn participant_departure_frees_capacity_without_changing_other_ownership() {
    let mut queue = VecDeque::new();
    for id in 0..super::super::protocol::MAX_QUEUE_ITEMS {
        let owner = if id % 2 == 0 { "alice" } else { "bob" };
        insert_item(&mut queue, Some("playing"), test_item(id, owner)).unwrap();
    }
    remove_owner_items(&mut queue, "alice");
    assert_eq!(queue.len(), super::super::protocol::MAX_QUEUE_ITEMS / 2);
    assert!(queue.iter().all(|item| item_owner(item) == Some("bob")));
    assert_eq!(
        insert_item(&mut queue, Some("playing"), test_item(100, "carol")),
        Ok(QueueInsert::Added)
    );
    assert_eq!(item_owner(queue.back().unwrap()), Some("carol"));
}

#[test]
fn batch_insert_is_atomic_ordered_and_idempotent() {
    let mut queue = VecDeque::from([test_item(0, "alice")]);
    let batch = vec![
        test_item(1, "bob"),
        test_item(2, "bob"),
        test_item(3, "bob"),
    ];
    assert_eq!(
        insert_batch(&mut queue, Some("playing"), batch.clone()),
        Ok(BatchInsert::Added)
    );
    assert_eq!(
        queue
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec!["alice:0", "bob:1", "bob:2", "bob:3"]
    );
    assert_eq!(
        insert_batch(&mut queue, Some("playing"), batch),
        Ok(BatchInsert::Duplicate)
    );
    assert_eq!(queue.len(), 4);
}

#[test]
fn batch_capacity_or_metadata_failure_leaves_queue_unchanged() {
    let mut queue = VecDeque::from(
        (0..super::super::protocol::MAX_QUEUE_ITEMS - 1)
            .map(|id| test_item(id, "alice"))
            .collect::<Vec<_>>(),
    );
    let original_ids = queue.iter().map(|item| item.id.clone()).collect::<Vec<_>>();
    assert_eq!(
        insert_batch(
            &mut queue,
            Some("playing"),
            vec![test_item(98, "bob"), test_item(99, "bob")],
        ),
        Err(BatchInsertError::Full)
    );
    assert_eq!(
        queue.iter().map(|item| item.id.clone()).collect::<Vec<_>>(),
        original_ids
    );

    let mut oversized = test_item(200, "bob");
    oversized.title = "\"".repeat(super::super::protocol::MAX_SERIALIZED_QUEUE_BYTES);
    assert_eq!(
        insert_batch(&mut queue, Some("playing"), vec![oversized]),
        Err(BatchInsertError::Full)
    );
    assert_eq!(
        queue.iter().map(|item| item.id.clone()).collect::<Vec<_>>(),
        original_ids
    );
}

#[test]
fn malformed_duplicate_batch_is_rejected_without_partial_commit() {
    let mut queue = VecDeque::from([test_item(0, "alice")]);
    let original_ids = queue.iter().map(|item| item.id.clone()).collect::<Vec<_>>();
    assert_eq!(
        insert_batch(
            &mut queue,
            Some("playing"),
            vec![test_item(1, "bob"), test_item(1, "bob")],
        ),
        Ok(BatchInsert::Duplicate)
    );
    assert_eq!(
        queue.iter().map(|item| item.id.clone()).collect::<Vec<_>>(),
        original_ids
    );
}

#[test]
fn metadata_growth_cannot_break_the_serialized_queue_budget() {
    let mut queue = VecDeque::from([test_item(1, "alice")]);
    let original = queue[0].clone();
    update_item_metadata(
        &mut queue,
        Some("playing"),
        &original.id,
        "alice",
        "\"".repeat(super::super::protocol::MAX_SERIALIZED_QUEUE_BYTES),
        original.source_url.clone(),
        None,
    );
    assert_eq!(queue[0].title, original.title);
}

#[test]
fn local_queue_items_do_not_publish_the_filesystem_path() {
    let item = queue_item("/private/music/song.mp3".into(), "user".into(), true);
    let value = serde_json::to_value(item).unwrap();
    assert_eq!(value["title"], "song");
    assert_eq!(value["sourceUrl"], "Local file");
    assert!(value.get("playbackSource").is_none());
    assert!(!value.to_string().contains("/private/music"));
}

#[test]
fn remote_queue_items_keep_the_locator_private() {
    let item = queue_item(
        "https://user:password@example.com/private/secret.mp3?token=hidden#fragment".into(),
        "user".into(),
        false,
    );
    assert_eq!(item.source_url, "https://example.com/");
    assert!(item.playback_source.contains("token=hidden"));
    let encoded = serde_json::to_string(&item).unwrap();
    assert!(!encoded.contains("private/secret"));
    assert!(!encoded.contains("secret.mp3"));
    assert!(!encoded.contains("hidden"));
    assert!(!encoded.contains("password"));
}

#[test]
fn removal_preserves_other_owners_and_their_queue_order() {
    let mut queue = VecDeque::from([
        queue_item("https://example.com/a".into(), "alice".into(), false),
        queue_item("https://example.com/b".into(), "bob".into(), false),
        queue_item("https://example.com/c".into(), "alice".into(), false),
    ]);
    remove_owner_items(&mut queue, "absent");
    assert_eq!(queue.len(), 3);
    remove_owner_items(&mut queue, "bob");
    assert_eq!(
        queue
            .iter()
            .map(|item| item.title.as_str())
            .collect::<Vec<_>>(),
        vec!["example.com", "example.com"]
    );
    assert!(queue.iter().all(|item| !item.local_file));
    assert_eq!(
        queue.front().unwrap().playback_source,
        "https://example.com/a"
    );
    assert_eq!(queue.front().unwrap().source_url, "https://example.com/");
}
