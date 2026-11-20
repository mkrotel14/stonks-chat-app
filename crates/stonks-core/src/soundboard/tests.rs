use super::*;

fn library() -> (tempfile::TempDir, SoundboardStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = SoundboardStore::new(&dir.path().join("data"), &dir.path().join("config"));
    (dir, store)
}

fn wav(path: &Path, frames: u32) {
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((36 + frames * 4).to_le_bytes());
    bytes.extend(b"WAVEfmt \x10\0\0\0\x01\0\x02\0");
    bytes.extend(24_000_u32.to_le_bytes());
    bytes.extend(96_000_u32.to_le_bytes());
    bytes.extend(b"\x04\0\x10\0data");
    bytes.extend((frames * 4).to_le_bytes());
    for _ in 0..frames {
        bytes.extend(12_000_i16.to_le_bytes());
        bytes.extend(4_000_i16.to_le_bytes());
    }
    fs::write(path, bytes).unwrap();
}

#[test]
fn real_wav_import_downmixes_resamples_and_round_trips_catalog() {
    let (dir, store) = library();
    assert!(store.list().unwrap().is_empty());
    let path = dir.path().join("Chime.WAV");
    wav(&path, 240);
    let clip = store.import(&path).unwrap();
    assert_eq!(clip.duration_ms, 10);
    assert_eq!(clip.name, "Chime");
    assert_eq!(decode(&store.clip_path(&clip)).unwrap(), vec![8000; 480]);
    let json: serde_json::Value =
        serde_json::from_slice(&fs::read(store.root.join("clips.json")).unwrap()).unwrap();
    assert_eq!(json[0]["fileName"], clip.file_name);
    assert_eq!(json[0]["durationMs"], 10);
    assert_eq!(store.rename(&clip.id, " Bell ").unwrap()[0].name, "Bell");
    store.set_join_clip(Some(&clip.id)).unwrap();
    #[cfg(all(
        feature = "native-voice",
        any(target_os = "linux", target_os = "windows")
    ))]
    {
        store.set_volume(0).unwrap();
        assert_eq!(
            store.join_pcm().unwrap(),
            Some((clip.id.clone(), vec![8000; 480]))
        );
    }
    assert!(store.delete(&clip.id).unwrap().is_empty());
    assert!(!store.clip_path(&clip).exists());
    #[cfg(all(
        feature = "native-voice",
        any(target_os = "linux", target_os = "windows")
    ))]
    assert_eq!(store.join_pcm().unwrap(), None);
    assert_eq!(
        store.settings().unwrap().join_soundboard_clip_id,
        Some(clip.id)
    );
}

#[test]
fn patches_only_requested_keys_and_validates_new_selection() {
    let (_dir, store) = library();
    assert_eq!(store.settings().unwrap(), SoundboardSettings::default());
    assert!(!store.preferences.exists());
    write_json(&store.preferences, &serde_json::json!({"theme":"system","future":{"enabled":true},"joinSoundboardClipId":"stale"})).unwrap();
    assert_eq!(store.set_join_clip(Some("missing")), Err(Error::NotFound));
    assert_eq!(store.set_join_clip(Some("../clip")), Err(Error::InvalidId));
    assert_eq!(store.set_volume(101), Err(Error::InvalidVolume));
    store.set_volume(45).unwrap();
    assert_eq!(
        store.settings().unwrap().join_soundboard_clip_id.as_deref(),
        Some("stale")
    );
    store.set_join_clip(None).unwrap();
    assert_eq!(
        store.preferences_json().unwrap(),
        serde_json::json!({"theme":"system","future":{"enabled":true},"joinSoundboardClipId":null,"soundboardVolume":45})
    );
    fs::write(&store.preferences, b"[]").unwrap();
    assert_eq!(store.set_volume(30), Err(Error::InvalidPreferences));
    assert_eq!(fs::read(&store.preferences).unwrap(), b"[]");
}

#[test]
fn rejects_unsafe_catalog_paths_and_oversized_or_invalid_audio() {
    let (dir, store) = library();
    let path = dir.path().join("Bad.wav");
    fs::write(&path, b"not audio").unwrap();
    assert_eq!(store.import(&path), Err(Error::InvalidAudio));
    wav(&path, 240_001);
    assert_eq!(store.import(&path), Err(Error::TooLong));
    fs::File::create(&path)
        .unwrap()
        .set_len(MAX_BYTES + 1)
        .unwrap();
    assert_eq!(store.import(&path), Err(Error::TooLarge));
    assert!(store.list().unwrap().is_empty());
    assert_eq!(fs::read_dir(store.root.join("clips")).unwrap().count(), 0);
    write_json(&store.root.join("clips.json"), &serde_json::json!([{"id":"clip-1","name":"Chime","fileName":"../outside.wav","durationMs":10}])).unwrap();
    assert_eq!(store.list(), Err(Error::InvalidCatalog));
}
