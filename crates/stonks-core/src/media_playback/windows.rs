//! Windows attachment backend: bounded HTTP download, Rodio audio, and FFmpeg video.
use super::*;
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Source as _};
use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::mpsc,
};

const DOWNLOAD_LIMIT: u64 = 256 * 1024 * 1024;
const PROBE_LIMIT: u64 = 16 * 1024 * 1024;
const FPS: f64 = 30.;
const TICK: Duration = Duration::from_millis(10);
// Do not let uploaded playlists or concat manifests reopen files/URLs.
const VIDEO_FORMATS: &str = "mov,mp4,m4a,3gp,3g2,mj2,matroska,webm,avi,mpeg,mpegts,ogg,flv,asf,gif";

type AudioDecoder = Decoder<BufReader<File>>;

fn decoder(path: &Path) -> Result<AudioDecoder, PlaybackError> {
    Decoder::try_from(File::open(path).map_err(|_| PlaybackError::Engine)?)
        .map_err(|_| PlaybackError::Unsupported)
}

async fn download(source: Source, limit: u64) -> Result<tempfile::TempPath, PlaybackError> {
    network::validate_headers(&source.headers)?;
    let url = validate_url(&source.url)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::custom(|hop| {
            if hop.previous().len() > 3 || validate_url(hop.url().as_str()).is_err() {
                hop.error("Invalid attachment redirect")
            } else {
                hop.follow()
            }
        }))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|_| PlaybackError::Unavailable)?;
    let mut request = client.get(url);
    for (name, value) in source.headers {
        request = request.header(name, value);
    }
    let mut response = request
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|_| PlaybackError::Unavailable)?;
    if response.content_length().is_some_and(|size| size > limit) {
        return Err(PlaybackError::Unsupported);
    }
    let (file, path) = tempfile::NamedTempFile::new()
        .map_err(|_| PlaybackError::Engine)?
        .into_parts();
    let mut file = tokio::fs::File::from_std(file);
    let mut size = 0;
    while let Some(bytes) = response
        .chunk()
        .await
        .map_err(|_| PlaybackError::Unavailable)?
    {
        size += bytes.len() as u64;
        if size > limit {
            return Err(PlaybackError::Unsupported);
        }
        file.write_all(&bytes)
            .await
            .map_err(|_| PlaybackError::Engine)?;
    }
    file.flush().await.map_err(|_| PlaybackError::Engine)?;
    Ok(path)
}

pub async fn probe_audio(source: Source) -> Result<AudioMetadata, PlaybackError> {
    static SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
    if source.kind != MediaKind::Audio {
        return Err(PlaybackError::Unsupported);
    }
    tokio::time::timeout(Duration::from_secs(25), async {
        let permit = SLOTS.acquire().await.map_err(|_| PlaybackError::Engine)?;
        let path = download(source, PROBE_LIMIT).await?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            decoder(&path)?
                .total_duration()
                .map(|duration| AudioMetadata {
                    duration: duration.as_secs_f64(),
                })
                .ok_or(PlaybackError::Unsupported)
        })
        .await
        .map_err(|_| PlaybackError::Engine)?
    })
    .await
    .map_err(|_| PlaybackError::Unavailable)?
}

fn tool(name: &str) -> PathBuf {
    if let Some(path) = std::env::var_os(format!("STONKS_{}_PATH", name.to_ascii_uppercase())) {
        return path.into();
    }
    let binary = format!("{name}.exe");
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for subdir in ["resources/media", "media", ""] {
                let candidate = dir.join(subdir).join(&binary);
                if candidate.is_file() {
                    return candidate;
                }
            }
        }
    }
    binary.into()
}

fn command(name: &str) -> Command {
    let mut child = Command::new(tool(name));
    child
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .args([
            "-v",
            "error",
            "-protocol_whitelist",
            "file",
            "-format_whitelist",
            VIDEO_FORMATS,
        ]);
    child
}

struct VideoInfo {
    width: u32,
    height: u32,
    duration: f64,
    audio: bool,
}

async fn inspect(path: &Path) -> Result<VideoInfo, PlaybackError> {
    let output = tokio::time::timeout(
        Duration::from_secs(15),
        command("ffprobe")
            .args([
                "-show_entries",
                "stream=codec_type,width,height:stream_disposition=attached_pic:format=duration",
                "-of",
                "json",
                "-i",
            ])
            .arg(path)
            .output(),
    )
    .await
    .map_err(|_| PlaybackError::Unsupported)?
    .map_err(|_| PlaybackError::VideoTools)?;
    if !output.status.success() {
        return Err(PlaybackError::Unsupported);
    }
    let data: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|_| PlaybackError::Unsupported)?;
    let streams = data["streams"]
        .as_array()
        .ok_or(PlaybackError::Unsupported)?;
    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1)
        .ok_or(PlaybackError::Unsupported)?;
    let size = |key: &str| {
        video[key]
            .as_u64()
            .filter(|&n| n > 0)
            .map(|n| n as f64)
            .ok_or(PlaybackError::Unsupported)
    };
    let (w, h) = (size("width")?, size("height")?);
    let scale = (f64::from(MAX_VIDEO_WIDTH) / w)
        .min(f64::from(MAX_VIDEO_HEIGHT) / h)
        .min(1.);
    let duration = data["format"]["duration"]
        .as_str()
        .and_then(|n| n.parse::<f64>().ok())
        .filter(|n| n.is_finite() && *n > 0.)
        .unwrap_or(0.);
    Ok(VideoInfo {
        width: ((w * scale) as u32).max(1),
        height: ((h * scale) as u32).max(1),
        duration,
        audio: streams.iter().any(|s| s["codec_type"] == "audio"),
    })
}

struct Video {
    frames: mpsc::Receiver<Vec<u8>>,
    task: tokio::task::JoinHandle<Result<(), PlaybackError>>,
    index: u64,
    started: Instant,
}
impl Video {
    fn start(path: &Path, info: &VideoInfo, seek: f64) -> Result<Self, PlaybackError> {
        let filter = format!("fps=30,scale={}:{},setsar=1", info.width, info.height);
        let mut child = command("ffmpeg")
            .args(["-ss", &format!("{seek:.6}"), "-noautorotate", "-i"])
            .arg(path)
            .args([
                "-map", "0:V:0", "-an", "-sn", "-dn", "-vf", &filter, "-threads", "1", "-pix_fmt",
                "bgra", "-f", "rawvideo", "pipe:1",
            ])
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|_| PlaybackError::VideoTools)?;
        let mut stdout = child.stdout.take().ok_or(PlaybackError::Engine)?;
        let (tx, frames) = mpsc::channel(2);
        let bytes = info.width as usize * info.height as usize * 4;
        let task = tokio::spawn(async move {
            loop {
                let mut frame = vec![0; bytes];
                match stdout.read_exact(&mut frame).await {
                    Ok(_) => {
                        if tx.send(frame).await.is_err() {
                            return Ok(());
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                    Err(_) => return Err(PlaybackError::Unsupported),
                }
            }
            if child.wait().await.is_ok_and(|status| status.success()) {
                Ok(())
            } else {
                Err(PlaybackError::Unsupported)
            }
        });
        Ok(Self {
            frames,
            task,
            index: 0,
            started: Instant::now(),
        })
    }
}
impl Drop for Video {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct Media {
    // Keep the file alive until the audio player and FFmpeg reader are dropped.
    player: Option<rodio::Player>,
    video: Option<Video>,
    sink: Option<MixerDeviceSink>,
    info: Option<VideoInfo>,
    path: tempfile::TempPath,
    duration: f64,
    start: f64,
    position: f64,
    anchor: Option<(Instant, f64)>,
    ended: bool,
}
impl Media {
    async fn open(source: Source) -> Result<Self, PlaybackError> {
        let kind = source.kind;
        let path = download(source, DOWNLOAD_LIMIT).await?;
        let info = if kind == MediaKind::Video {
            Some(inspect(&path).await?)
        } else {
            None
        };
        let audio = info.as_ref().is_none_or(|info| info.audio);
        let duration = if let Some(info) = &info {
            info.duration
        } else {
            decoder(&path)?
                .total_duration()
                .map_or(0., |d| d.as_secs_f64())
        };
        let sink = if audio {
            let mut sink =
                DeviceSinkBuilder::open_default_sink().map_err(|_| PlaybackError::Engine)?;
            sink.log_on_drop(false);
            Some(sink)
        } else {
            None
        };
        let mut media = Self {
            player: None,
            video: None,
            sink,
            info,
            path,
            duration,
            start: 0.,
            position: 0.,
            anchor: None,
            ended: false,
        };
        media.restart(0.)?;
        Ok(media)
    }

    fn restart(&mut self, position: f64) -> Result<(), PlaybackError> {
        let position = if self.duration > 0. {
            position.min((self.duration - 0.001).max(0.))
        } else {
            position
        };
        let seek = Duration::try_from_secs_f64(position).map_err(|_| PlaybackError::Seek)?;
        let player = if let Some(sink) = &self.sink {
            let mut audio = decoder(&self.path)?;
            if !seek.is_zero() {
                audio.try_seek(seek).map_err(|_| PlaybackError::Seek)?;
            }
            let player = rodio::Player::connect_new(sink.mixer());
            player.pause();
            player.append(audio);
            Some(player)
        } else {
            None
        };
        let video = self
            .info
            .as_ref()
            .map(|info| Video::start(&self.path, info, position))
            .transpose()?;
        self.player = player;
        self.video = video;
        self.position = position;
        self.start = position;
        self.anchor = None;
        self.ended = false;
        Ok(())
    }

    async fn tick(
        &mut self,
        playing: bool,
        volume: f64,
        snapshot: &mut Snapshot,
    ) -> Result<(), PlaybackError> {
        if self.ended {
            snapshot.volume = volume;
            return Ok(());
        }
        // Wait for the first frame before starting the audio or advancing time.
        let waiting = self.video.as_ref().is_some_and(|video| video.index == 0);
        let advance = playing && !waiting;
        if let Some(player) = &self.player {
            player.set_volume(volume as f32);
            if advance {
                player.play();
            } else {
                player.pause();
            }
            if !player.empty() {
                self.position = self.start + player.get_pos().as_secs_f64();
            } else if self.anchor.is_none() && advance {
                self.anchor = Some((Instant::now(), self.position));
            }
        } else if self.anchor.is_none() && advance {
            self.anchor = Some((Instant::now(), self.position));
        }
        if let Some((time, position)) = self.anchor {
            self.position = position + time.elapsed().as_secs_f64();
            if !playing {
                self.anchor = None;
            }
        }
        let mut video_ended = self.video.is_none();
        if let (Some(video), Some(info)) = (&mut self.video, &self.info) {
            while self.start + video.index as f64 / FPS <= self.position + 0.010 {
                match video.frames.try_recv() {
                    Ok(bgra) => {
                        video.index += 1;
                        snapshot.frame = Some(VideoFrame {
                            width: info.width,
                            height: info.height,
                            bgra: bgra.into(),
                            serial: snapshot.revision.wrapping_add(1),
                        });
                    }
                    Err(mpsc::error::TryRecvError::Empty) => {
                        if waiting && video.started.elapsed() > Duration::from_secs(15) {
                            return Err(PlaybackError::Unsupported);
                        }
                        break;
                    }
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        if !video_ended && video.task.is_finished() {
                            (&mut video.task)
                                .await
                                .map_err(|_| PlaybackError::Engine)??;
                            if video.index == 0 {
                                return Err(PlaybackError::Unsupported);
                            }
                            video_ended = true;
                        }
                        break;
                    }
                }
            }
        }
        if video_ended && self.video.is_some() {
            self.video = None;
        }
        let ended = self.player.as_ref().is_none_or(|player| player.empty()) && video_ended;
        self.ended = ended;
        snapshot.position = if ended && self.duration > 0. {
            self.duration
        } else if self.duration > 0. {
            self.position.min(self.duration)
        } else {
            self.position
        };
        snapshot.duration = self.duration;
        snapshot.volume = volume;
        snapshot.status = if ended {
            PlaybackStatus::Ended
        } else if waiting && snapshot.frame.is_none() {
            PlaybackStatus::Loading
        } else if playing {
            PlaybackStatus::Playing
        } else {
            PlaybackStatus::Paused
        };
        Ok(())
    }
}

pub(super) fn run(shared: Arc<Shared>) {
    match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime.block_on(run_async(shared)),
        Err(_) => {
            if let Ok(mut snapshot) = shared.snapshot.lock() {
                snapshot.error = Some(PlaybackError::Engine);
                snapshot.status = PlaybackStatus::Failed;
            }
        }
    }
}

async fn run_async(shared: Arc<Shared>) {
    let mut generation = 0;
    let mut media: Option<Media> = None;
    let mut playing = false;
    let mut interval = tokio::time::interval(TICK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    'worker: loop {
        interval.tick().await;
        if shared.shutdown.load(Ordering::Acquire) {
            break;
        }
        let (next, source, want_playing, volume, seek) = {
            let Ok(mut desired) = shared.desired.lock() else {
                break;
            };
            let source = if generation != desired.generation {
                desired.source.take()
            } else {
                None
            };
            let seek = if generation == desired.generation {
                desired.seek.take()
            } else {
                None
            };
            (
                desired.generation,
                source,
                desired.playing,
                desired.volume,
                seek,
            )
        };
        if generation != next {
            generation = next;
            media = None;
            playing = false;
            let Some(source) = source else {
                continue;
            };
            let open = Media::open(source);
            tokio::pin!(open);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if shared.shutdown.load(Ordering::Acquire) { break 'worker; }
                        if shared.desired.lock().map_or(true, |d| d.generation != generation) { continue 'worker; }
                    }
                    result = &mut open => {
                        match result {
                            Ok(opened) => media = Some(opened),
                            Err(error) => update(&shared, generation, |snapshot| fail(snapshot, error)),
                        }
                        continue 'worker;
                    }
                }
            }
        }
        let Some(active) = &mut media else {
            continue;
        };
        let mut snapshot = match shared.snapshot.lock() {
            Ok(s) => s.clone(),
            Err(_) => break,
        };
        if let Some(position) =
            seek.or_else(|| (want_playing && !playing && active.ended).then_some(0.))
        {
            match active.restart(position) {
                Ok(()) => {
                    snapshot.frame = None;
                    snapshot.error = None;
                }
                Err(error) => {
                    snapshot.error = Some(error);
                }
            }
        }
        playing = want_playing;
        if let Err(error) = active.tick(playing, volume, &mut snapshot).await {
            fail(&mut snapshot, error);
            media = None;
        }
        update(&shared, generation, |current| *current = snapshot);
    }
}

fn fail(snapshot: &mut Snapshot, error: PlaybackError) {
    snapshot.status = PlaybackStatus::Failed;
    snapshot.error = Some(error);
    snapshot.frame = None;
}

fn update(shared: &Shared, generation: u64, update: impl FnOnce(&mut Snapshot)) {
    let Ok(desired) = shared.desired.lock() else {
        return;
    };
    if desired.generation != generation {
        return;
    }
    if let Ok(mut snapshot) = shared.snapshot.lock() {
        update(&mut snapshot);
        snapshot.revision = snapshot.revision.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Opt in on a Windows host with the media tools installed; no audio endpoint needed.
    #[tokio::test]
    #[ignore = "requires FFmpeg and FFprobe"]
    async fn silent_video_reports_metadata_and_decodes_bgra() {
        let fixture = tempfile::NamedTempFile::new().unwrap().into_temp_path();
        let output = Command::new(tool("ffmpeg"))
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=red:s=64x32:r=30",
                "-t",
                "1",
                "-c:v",
                "mpeg4",
                "-f",
                "mp4",
            ])
            .arg(&fixture)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let info = inspect(&fixture).await.unwrap();
        assert_eq!((info.width, info.height, info.audio), (64, 32, false));
        assert!((info.duration - 1.).abs() < 0.1);
        let mut video = Video::start(&fixture, &info, 0.25).unwrap();
        let pixels = tokio::time::timeout(Duration::from_secs(5), video.frames.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(pixels.len(), 64 * 32 * 4);
        assert!(pixels[2] > 200 && pixels[0] < 20 && pixels[3] == 255);
    }
}
