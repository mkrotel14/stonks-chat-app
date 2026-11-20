use super::*;
use std::{
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, AtomicUsize},
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    directory: PathBuf,
    base: String,
    requests: Arc<AtomicUsize>,
    headers: Arc<Mutex<Vec<String>>>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
impl Fixture {
    async fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "stonks-playback-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let files = directory.clone();
        let (audio, video) = tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&files).unwrap();
            let mp3 = files.join("fixture.mp3");
            let mp4 = files.join("fixture.mp4");
            let output = Command::new("ffmpeg")
                .args([
                    "-nostdin",
                    "-loglevel",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=440:duration=2",
                    "-c:a",
                    "libmp3lame",
                    "-y",
                ])
                .arg(&mp3)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "ffmpeg MP3 fixture failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = Command::new("ffmpeg")
                .args([
                    "-nostdin",
                    "-loglevel",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "testsrc=size=64x48:rate=12:duration=2",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=330:duration=2",
                    "-c:v",
                    "libx264",
                    "-threads",
                    "1",
                    "-pix_fmt",
                    "yuv420p",
                    "-c:a",
                    "aac",
                    "-shortest",
                    "-movflags",
                    "+faststart",
                    "-y",
                ])
                .arg(&mp4)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "ffmpeg MP4 fixture failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            (
                Arc::<[u8]>::from(std::fs::read(mp3).unwrap()),
                Arc::<[u8]>::from(std::fs::read(mp4).unwrap()),
            )
        })
        .await
        .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let count = requests.clone();
        let headers = Arc::new(Mutex::new(vec![]));
        let seen = headers.clone();
        let server = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted=listener.accept()=>{let Ok((mut socket,_))=accepted else{break;};if connections.len()>=4{continue;}let audio=audio.clone();let video=video.clone();let count=count.clone();let seen=seen.clone();
                        connections.spawn(async move{
                            let mut request=vec![];
                            loop{let mut bytes=[0;2048];let Ok(n)=socket.read(&mut bytes).await else{return;};if n==0{return;}request.extend_from_slice(&bytes[..n]);if request.windows(4).any(|part|part==b"\r\n\r\n"){break;}if request.len()>16384{return;}}
                            let request=String::from_utf8_lossy(&request).into_owned();count.fetch_add(1,Ordering::Relaxed);if let Ok(mut headers)=seen.lock(){if headers.len()<64{headers.push(request.clone());}}
                            let path=request.lines().next().and_then(|line|line.split_whitespace().nth(1)).unwrap_or_default();
                            if path=="/probe-slow"&&request.starts_with("HEAD "){let _=socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nContent-Length: 100000\r\nConnection: close\r\n\r\n").await;return;}
                            if path=="/probe-slow"{futures_util::future::pending::<()>().await;return;}
                            if path=="/slow"{futures_util::future::pending::<()>().await;return;}
                            if path=="/redirect"||path=="/unsafe"{let location=if path=="/unsafe"{"file:///must-not-be-opened"}else{"/video.mp4"};let _=socket.write_all(format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await;return;}
                            let(data,content_type)=if path=="/audio.mp3"{(audio,"audio/mpeg")}else if path=="/bad"{(Arc::from(&b"not a media file"[..]),"application/octet-stream")}else{(video,"video/mp4")};
                            let range=request.lines().find_map(|line|{let(name,value)=line.split_once(':')?;name.eq_ignore_ascii_case("range").then_some(value.trim())});
                            let(start,end)=range.and_then(|value|value.strip_prefix("bytes=")).and_then(|value|value.split_once('-')).and_then(|(start,end)|Some((start.parse::<usize>().ok()?,end.parse::<usize>().ok().unwrap_or(data.len()-1)))).map(|(start,end)|(start.min(data.len()),end.min(data.len()-1))).unwrap_or((0,data.len()-1));
                            let partial=range.is_some();let length=end.saturating_add(1).saturating_sub(start);
                            let response=format!("HTTP/1.1 {}\r\nContent-Type: {content_type}\r\nContent-Length: {length}\r\nAccept-Ranges: bytes\r\n{}Connection: close\r\n\r\n",if partial{"206 Partial Content"}else{"200 OK"},if partial{format!("Content-Range: bytes {start}-{end}/{}\r\n",data.len())}else{String::new()});
                            let _=socket.write_all(response.as_bytes()).await;if !request.starts_with("HEAD ")&&length>0{let _=socket.write_all(&data[start..=end]).await;}
                        });
                    },Some(_)=connections.join_next(),if !connections.is_empty()=>{}
                }
            }
        });
        Self {
            directory,
            base,
            requests,
            headers,
            server,
        }
    }
    fn source(&self, key: &str, path: &str, kind: MediaKind, autoplay: bool) -> Source {
        Source::new(
            key.into(),
            format!("{}{path}", self.base),
            kind,
            autoplay,
            vec![],
        )
        .unwrap()
    }
}
async fn wait(player: &Player, predicate: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        let snapshot = player.snapshot();
        assert_ne!(
            snapshot.status,
            PlaybackStatus::Failed,
            "pipeline failed: {:?}",
            snapshot.error
        );
        if predicate(&snapshot) {
            return snapshot;
        }
        assert!(Instant::now() < deadline, "timed out: {snapshot:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
async fn finished(shared: &Arc<Shared>) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !shared.finished.load(Ordering::Acquire) {
        assert!(Instant::now() < deadline, "owned worker failed to stop");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
#[test]
fn only_http_media_and_public_resolver_headers_are_accepted() {
    for url in [
        "file:///tmp/video.mp4",
        "data:video/mp4;base64,AAAA",
        "https://user:pass@example.com/v",
        "ftp://example.com/file",
        "https://user@example.com/v",
    ] {
        assert!(Source::new("key".into(), url.into(), MediaKind::Video, false, vec![]).is_err());
    }
    for headers in [
        vec![("Authorization".into(), "Bearer token".into())],
        vec![("Cookie".into(), "private".into())],
        vec![("User-Agent".into(), "value\r\nInjected: yes".into())],
    ] {
        assert!(Source::new(
            "key".into(),
            "https://example.com/video.mp4".into(),
            MediaKind::Video,
            false,
            headers
        )
        .is_err());
    }
}
#[tokio::test]
async fn redirects_are_validated_without_cookies_or_session_credentials() {
    let fixture = Fixture::new().await;
    let source = resolve_source(fixture.source("video", "/redirect", MediaKind::Video, false))
        .await
        .unwrap();
    assert!(source.url.ends_with("/video.mp4"));
    assert_eq!(
        resolve_source(fixture.source("unsafe", "/unsafe", MediaKind::Video, false))
            .await
            .unwrap_err(),
        PlaybackError::InvalidUrl
    );
    for request in fixture.headers.lock().unwrap().iter() {
        let request = request.to_lowercase();
        assert!(!request.contains("authorization:"));
        assert!(!request.contains("cookie:"));
        assert!(!request.contains("x-session-token:"));
    }
}
#[tokio::test]
async fn actual_mp3_pipeline_plays_pauses_seeks_ends_and_replays() {
    let fixture = Fixture::new().await;
    let player = Player::with_audio(worker::AudioOutput::Silent).unwrap();
    assert_eq!(player.snapshot().status, PlaybackStatus::Idle);
    assert_eq!(fixture.requests.load(Ordering::Relaxed), 0);
    player
        .load(fixture.source("audio", "/audio.mp3", MediaKind::Audio, true))
        .unwrap();
    let loaded = wait(&player, |state| {
        state.status == PlaybackStatus::Playing && state.position > 0.15 && state.duration > 1.
    })
    .await;
    player.set_volume("audio", 0.3);
    wait(&player, |state| (state.volume - 0.3).abs() < 0.001).await;
    player.pause("audio");
    let paused = wait(&player, |state| state.status == PlaybackStatus::Paused).await;
    tokio::time::sleep(Duration::from_millis(130)).await;
    assert!((player.snapshot().position - paused.position).abs() < 0.08);
    let stable = player.snapshot();
    tokio::time::sleep(Duration::from_millis(130)).await;
    assert_eq!(
        player.snapshot().revision,
        stable.revision,
        "paused audio must not publish unchanged frames"
    );
    player.seek("audio", 1.);
    wait(&player, |state| (state.position - 1.).abs() < 0.12).await;
    player.play("audio");
    wait(&player, |state| {
        state.status == PlaybackStatus::Playing && state.position > 1.1
    })
    .await;
    player.seek("audio", loaded.duration - 0.15);
    wait(&player, |state| state.status == PlaybackStatus::Ended).await;
    player.seek("audio", 1.);
    let after_end = wait(&player, |state| {
        state.status == PlaybackStatus::Paused && (state.position - 1.).abs() < 0.12
    })
    .await;
    tokio::time::sleep(Duration::from_millis(130)).await;
    assert!(
        (player.snapshot().position - after_end.position).abs() < 0.08,
        "seeking after EOS must not autoplay"
    );
    player.seek("audio", loaded.duration - 0.15);
    player.play("audio");
    wait(&player, |state| state.status == PlaybackStatus::Ended).await;
    player.play("audio");
    wait(&player, |state| {
        state.status == PlaybackStatus::Playing && state.position < 0.5
    })
    .await;
    player.stop();
    assert_eq!(player.snapshot().status, PlaybackStatus::Idle);
    let shared = player.shared.clone();
    drop(player);
    finished(&shared).await;
}
#[tokio::test]
async fn actual_mp4_preview_stays_paused_and_delivers_bounded_bgra_frames_then_plays() {
    let fixture = Fixture::new().await;
    let player = Player::with_audio(worker::AudioOutput::Silent).unwrap();
    player
        .load(fixture.source("video", "/video.mp4", MediaKind::Video, false))
        .unwrap();
    let paused = wait(&player, |state| {
        state.status == PlaybackStatus::Paused && state.frame.is_some() && state.duration > 1.
    })
    .await;
    let frame = paused.frame.unwrap();
    assert_eq!((frame.width, frame.height), (64, 48));
    assert_eq!(frame.bgra.len(), 64 * 48 * 4);
    assert!(frame.bgra.len() <= MAX_FRAME_BYTES);
    assert!(frame
        .bgra
        .chunks_exact(4)
        .any(|pixel| pixel[3] == 255 && pixel[0] != pixel[2]));
    player.play("video");
    let moving = wait(&player, |state| {
        state.status == PlaybackStatus::Playing
            && state
                .frame
                .as_ref()
                .is_some_and(|next| next.serial > frame.serial + 2)
    })
    .await;
    assert!(moving.position > 0.);
    player
        .load(fixture.source("replacement", "/audio.mp3", MediaKind::Audio, true))
        .unwrap();
    player.seek("video", 50.);
    player.pause("video");
    let replacement = wait(&player, |state| {
        state.key.as_deref() == Some("replacement") && state.status == PlaybackStatus::Playing
    })
    .await;
    assert!(replacement.frame.is_none());
    let shared = player.shared.clone();
    drop(player);
    finished(&shared).await;
}
#[tokio::test]
async fn resolver_headers_reach_real_http_playback_without_credentials() {
    let fixture = Fixture::new().await;
    let player = Player::with_audio(worker::AudioOutput::Silent).unwrap();
    let mut source = fixture.source("headers", "/video.mp4", MediaKind::Video, false);
    source.headers = vec![
        ("User-Agent".into(), "Stonks-Local-Fixture".into()),
        ("Referer".into(), format!("{}/watch", fixture.base)),
    ];
    let source = resolve_source(source).await.unwrap();
    player.load(source).unwrap();
    wait(&player, |state| state.frame.is_some()).await;
    let headers = fixture.headers.lock().unwrap();
    let get = headers
        .iter()
        .find(|request| request.starts_with("GET /video.mp4"))
        .unwrap()
        .to_lowercase();
    assert!(get.contains("user-agent: stonks-local-fixture"));
    assert!(get.contains(&format!("referer: {}/watch", fixture.base)));
    assert!(!get.contains("cookie:"));
    assert!(!get.contains("x-session-token:"));
}
#[tokio::test]
async fn invalid_media_can_retry_and_drop_while_loading_stops_the_worker() {
    let fixture = Fixture::new().await;
    let player = Player::with_audio(worker::AudioOutput::Silent).unwrap();
    player
        .load(fixture.source("retry", "/bad", MediaKind::Video, false))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(4);
    while player.snapshot().status != PlaybackStatus::Failed {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    player
        .load(fixture.source("retry", "/video.mp4", MediaKind::Video, false))
        .unwrap();
    wait(&player, |state| {
        state.status == PlaybackStatus::Paused && state.frame.is_some()
    })
    .await;
    player
        .load(fixture.source("pending", "/slow", MediaKind::Video, true))
        .unwrap();
    let before = fixture.requests.load(Ordering::Relaxed);
    let deadline = Instant::now() + Duration::from_secs(3);
    while fixture.requests.load(Ordering::Relaxed) == before {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let shared = player.shared.clone();
    drop(player);
    finished(&shared).await;
}

#[tokio::test]
async fn audio_metadata_preloads_without_activating_or_interrupting_playback() {
    let fixture = Fixture::new().await;
    let player = Player::with_audio(worker::AudioOutput::Silent).unwrap();
    player
        .load(fixture.source("active", "/audio.mp3", MediaKind::Audio, false))
        .unwrap();
    let before = wait(&player, |snapshot| {
        snapshot.status == PlaybackStatus::Paused && snapshot.duration > 1.
    })
    .await;
    let metadata = probe_audio(fixture.source("metadata", "/audio.mp3", MediaKind::Audio, true))
        .await
        .unwrap();
    assert!((metadata.duration - 2.).abs() < 0.1);
    let after = player.snapshot();
    assert_eq!(after.key.as_deref(), Some("active"));
    assert_eq!(after.status, PlaybackStatus::Paused);
    assert!((after.position - before.position).abs() < 0.01);
    assert_eq!(
        probe_audio(fixture.source("video", "/video.mp4", MediaKind::Video, false))
            .await
            .unwrap_err(),
        PlaybackError::Unsupported
    );
    assert_eq!(
        probe_audio(fixture.source("unsafe", "/unsafe", MediaKind::Audio, false))
            .await
            .unwrap_err(),
        PlaybackError::InvalidUrl
    );
    assert!(
        probe_audio(fixture.source("bad", "/bad", MediaKind::Audio, false))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cancelled_audio_metadata_stops_a_stalled_preroll() {
    let fixture = Fixture::new().await;
    let source = fixture.source("cancelled-metadata", "/probe-slow", MediaKind::Audio, false);
    let cancelled = Arc::new(AtomicBool::new(false));
    let shutdown = cancelled.clone();
    let worker = tokio::task::spawn_blocking(move || worker::probe_audio(source, &shutdown));
    let deadline = Instant::now() + Duration::from_secs(2);
    while !fixture
        .headers
        .lock()
        .unwrap()
        .iter()
        .any(|request| request.starts_with("GET /probe-slow"))
    {
        assert!(
            Instant::now() < deadline,
            "probe did not reach local request"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    cancelled.store(true, Ordering::Release);
    let result = tokio::time::timeout(Duration::from_secs(2), worker)
        .await
        .expect("cancelled probe must release pipeline promptly")
        .unwrap();
    assert_eq!(result.unwrap_err(), PlaybackError::Unavailable);
}

#[tokio::test]
async fn metadata_future_cancellation_releases_its_bounded_worker_slot() {
    let fixture = Fixture::new().await;
    let slots = Arc::new(tokio::sync::Semaphore::new(1));
    let blocked = tokio::spawn(probe::probe_with_slots(
        fixture.source("blocked", "/probe-slow", MediaKind::Audio, false),
        slots.clone(),
    ));
    let deadline = Instant::now() + Duration::from_secs(2);
    while !fixture
        .headers
        .lock()
        .unwrap()
        .iter()
        .any(|request| request.starts_with("GET /probe-slow"))
    {
        assert!(
            Instant::now() < deadline,
            "probe did not reach local request"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(slots.available_permits(), 0);
    let queued = tokio::spawn(probe::probe_with_slots(
        fixture.source("next", "/audio.mp3", MediaKind::Audio, false),
        slots.clone(),
    ));
    // The next probe has no permit and therefore cannot even start resolving.
    tokio::task::yield_now().await;
    assert!(!fixture
        .headers
        .lock()
        .unwrap()
        .iter()
        .any(|request| request.contains("/audio.mp3")));
    blocked.abort();
    assert!(blocked.await.unwrap_err().is_cancelled());
    let metadata = tokio::time::timeout(Duration::from_secs(3), queued)
        .await
        .expect("cancelled worker must release its owned slot")
        .unwrap()
        .unwrap();
    assert!((metadata.duration - 2.).abs() < 0.1);
    assert_eq!(slots.available_permits(), 1);
}
