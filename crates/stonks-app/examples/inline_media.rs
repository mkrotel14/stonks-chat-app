//! Offline production-player fixture. Audio/video contain digital silence.
//! No saved account or network outside its loopback HTTP server is used.
#![allow(dead_code)]
use gpui_kit::*;
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};
use stonks_core::{
    connection::AppSnapshot,
    http::{ContextError, HttpPolicy, SessionAccess},
    messages::models::TimelineAttachment,
    server::{
        models::{ServerCapabilities, ServerEndpoints},
        ServerSelection,
    },
};
use stonks_ui::{
    components::{Button, ButtonVariant},
    fonts,
    theme::CurrentTheme,
    Theme,
};
#[path = "../src/features/conversation/media/media_playback.rs"]
mod media_playback;
#[path = "../src/features/conversation/media/media_player/mod.rs"]
mod media_player;
#[path = "../src/runtime.rs"]
mod runtime;
#[path = "../src/session/mod.rs"]
mod session;
use media_playback::MediaPlayback;
use media_player::MediaPlayers;
use session::Session;

struct LocalSelection(ServerSelection);
impl SessionAccess for LocalSelection {
    fn server_selection(&self) -> Result<Option<ServerSelection>, ContextError> {
        Ok(Some(self.0.clone()))
    }
    fn token(&self, _: &ServerSelection) -> Result<Option<String>, ContextError> {
        panic!("offline playback must never request credentials")
    }
}
struct LocalServer {
    base: String,
    stop: Arc<AtomicBool>,
    repair: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    directory: PathBuf,
    sizes: HashMap<&'static str, u64>,
}
impl LocalServer {
    fn new() -> Self {
        let directory =
            std::env::temp_dir().join(format!("stonks-inline-media-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("fixture directory");
        let mp3 = directory.join("silent.mp3");
        let mp4 = directory.join("silent.mp4");
        let audio = Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "anullsrc=r=44100:cl=mono",
                "-t",
                "8",
                "-c:a",
                "libmp3lame",
                "-y",
            ])
            .arg(&mp3)
            .output()
            .expect("ffmpeg fixture encoder");
        assert!(audio.status.success(), "silent MP3 fixture failed");
        let video = Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=480x270:rate=24:duration=8",
                "-f",
                "lavfi",
                "-i",
                "anullsrc=r=44100:cl=mono",
                "-t",
                "8",
                "-c:v",
                "libx264",
                "-threads",
                "1",
                "-preset",
                "ultrafast",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-movflags",
                "+faststart",
                "-y",
            ])
            .arg(&mp4)
            .output()
            .expect("ffmpeg fixture encoder");
        assert!(video.status.success(), "silent MP4 fixture failed");
        let audio = Arc::<[u8]>::from(std::fs::read(mp3).expect("silent MP3"));
        let video = Arc::<[u8]>::from(std::fs::read(mp4).expect("silent MP4"));
        let sizes = HashMap::from([
            ("/silent.mp3", audio.len() as u64),
            ("/silent.mp4", video.len() as u64),
            ("/recover.mp4", video.len() as u64),
        ]);
        let listener = TcpListener::bind("127.0.0.1:0").expect("fixture HTTP listener");
        listener.set_nonblocking(true).expect("nonblocking fixture");
        let base = format!("http://{}", listener.local_addr().expect("fixture address"));
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let repair = Arc::new(AtomicBool::new(false));
        let repaired = repair.clone();
        let worker = thread::spawn(move || {
            let mut clients: Vec<thread::JoinHandle<()>> = vec![];
            while !stopping.load(Ordering::Acquire) {
                let mut index = 0;
                while index < clients.len() {
                    if clients[index].is_finished() {
                        let _ = clients.swap_remove(index).join();
                    } else {
                        index += 1;
                    }
                }
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        if clients.len() >= 8 {
                            continue;
                        }
                        let audio = audio.clone();
                        let video = video.clone();
                        let repaired = repaired.clone();
                        clients.push(thread::spawn(move||{
                            let _=socket.set_read_timeout(Some(Duration::from_secs(2)));let _=socket.set_write_timeout(Some(Duration::from_secs(2)));
                            let mut request=vec![];loop{let mut bytes=[0;2048];let Ok(n)=socket.read(&mut bytes)else{return;};if n==0{return;}request.extend_from_slice(&bytes[..n]);if request.windows(4).any(|part|part==b"\r\n\r\n"){break;}if request.len()>16384{return;}}
                            let request=String::from_utf8_lossy(&request);let path=request.split_whitespace().nth(1).unwrap_or_default();
                            let(data,kind)=match path{"/silent.mp3"=>(audio,"audio/mpeg"),"/recover.mp4" if !repaired.load(Ordering::Acquire)=>(Arc::from(&b"fixture decode failure"[..]),"application/octet-stream"),_=>(video,"video/mp4")};
                            let range=request.lines().find_map(|line|{let(name,value)=line.split_once(':')?;name.eq_ignore_ascii_case("range").then_some(value.trim())});
                            let(start,end)=range.and_then(|value|value.strip_prefix("bytes=")).and_then(|value|value.split_once('-')).and_then(|(start,end)|Some((start.parse::<usize>().ok()?,end.parse::<usize>().ok().unwrap_or(data.len()-1)))).map(|(start,end)|(start.min(data.len()),end.min(data.len()-1))).unwrap_or((0,data.len()-1));
                            let length=end.saturating_add(1).saturating_sub(start);let _=write!(socket,"HTTP/1.1 {}\r\nContent-Type: {kind}\r\nContent-Length: {length}\r\nAccept-Ranges: bytes\r\n{}Connection: close\r\n\r\n",if range.is_some(){"206 Partial Content"}else{"200 OK"},if range.is_some(){format!("Content-Range: bytes {start}-{end}/{}\r\n",data.len())}else{String::new()});
                            if !request.starts_with("HEAD ")&&length>0{let _=socket.write_all(&data[start..=end]);}
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => break,
                }
            }
            for client in clients {
                let _ = client.join();
            }
        });
        Self {
            base,
            stop,
            repair,
            worker: Some(worker),
            directory,
            sizes,
        }
    }
}
impl Drop for LocalServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
fn snapshot() -> AppSnapshot {
    AppSnapshot {
        current_user: serde_json::from_value(
            serde_json::json!({"_id":"fixture-user","username":"Offline"}),
        )
        .expect("fixture user"),
        users: vec![],
        servers: vec![],
        channels: vec![],
        members: vec![],
        unreads: vec![],
        voice_states: vec![],
    }
}
struct Preview {
    session: Entity<Session>,
    playback: Entity<MediaPlayback>,
    players: Entity<MediaPlayers>,
    server: LocalServer,
    channel: bool,
    status: String,
    scheduled: Option<Task<()>>,
}
impl Preview {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let server = LocalServer::new();
        let selection = ServerSelection {
            deployment_url: server.base.clone(),
            display_name: "Offline playback".into(),
            official: false,
            api_version: "fixture".into(),
            capabilities: ServerCapabilities::default(),
            endpoints: ServerEndpoints {
                api: server.base.clone(),
                websocket: "ws://127.0.0.1/unused".into(),
                autumn: None,
                january: None,
                gifbox: None,
                livekit_enabled: false,
            },
        };
        let policy = Arc::new(
            HttpPolicy::new(Arc::new(LocalSelection(selection.clone())))
                .expect("fixture HTTP policy"),
        );
        let session = cx.new(|_| {
            let mut session = Session::new(policy.clone());
            session.server = Some(selection);
            session.snapshot = Some(snapshot());
            session
        });
        let playback = cx.new(|cx| MediaPlayback::new(session.clone(), cx));
        playback.update(cx, |playback, cx| {
            playback.set_channel(Some("fixture-a".into()), cx)
        });
        let players = cx.new(|cx| MediaPlayers::new(playback.clone(), policy, window, cx));
        Self {
            session,
            playback,
            players,
            server,
            channel: false,
            status: "Digital silence; real MP3/H.264/AAC decode and production controls.".into(),
            scheduled: None,
        }
    }
    fn file(&self, id: &str, path: &'static str, kind: &str) -> TimelineAttachment {
        TimelineAttachment {
            id: id.into(),
            filename: if kind == "Audio" {
                "silent-audio.mp3".into()
            } else {
                format!("{id}.mp4")
            },
            content_type: if kind == "Audio" {
                "audio/mpeg".into()
            } else {
                "video/mp4".into()
            },
            size: self.server.sizes[path],
            kind: kind.into(),
            width: (kind == "Video").then_some(480),
            height: (kind == "Video").then_some(270),
            url: Some(format!("{}{path}", self.server.base)),
            original_url: Some(format!("{}{path}", self.server.base)),
        }
    }
}
impl Render for Preview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(cx);
        let width = (f32::from(window.viewport_size().width) - 48.)
            .min(520.)
            .max(1.);
        let mut body=div().id("inline-media-fixture").size_full().overflow_y_scroll().flex().flex_col().gap(px(theme.space4)).p(px(theme.space6)).font_family(fonts::FAMILY).bg(theme.canvas).text_color(theme.text)
            .child("Production inline media — offline fixture")
            .child("Audio metadata preloads; video waits for Preview. Seek, volume, pause, end/replay and retry are real GStreamer operations.")
            .child(div().flex().flex_wrap().gap(px(theme.space2))
                .child(Button::new("fixture-stop",theme).variant(ButtonVariant::Secondary).label("Stop").on_click(cx.listener(|this,_,_,cx|{this.players.update(cx,|players,cx|players.reset(cx));this.status="Stopped; all player rows reset.".into();cx.notify();})))
                .child(Button::new("fixture-channel",theme).variant(ButtonVariant::Secondary).label("Switch channel").on_click(cx.listener(|this,_,_,cx|{this.channel=!this.channel;let channel=if this.channel{"fixture-b"}else{"fixture-a"};this.playback.update(cx,|playback,cx|playback.set_channel(Some(channel.into()),cx));this.players.update(cx,|players,cx|players.reset(cx));this.status=format!("Channel changed to {channel}; playback stopped.");cx.notify();})))
                .child(Button::new("fixture-logout",theme).variant(ButtonVariant::Secondary).label("Mock logout").on_click(cx.listener(|this,_,_,cx|{this.session.update(cx,|session,cx|{session.snapshot=None;cx.notify();});this.status="Mock account cleared; playback stopped.".into();cx.notify();})))
                .child(Button::new("fixture-scheduled-logout",theme).variant(ButtonVariant::Secondary).label("Mock logout in 2s").on_click(cx.listener(|this,_,_,cx|{
                    this.status="Mock logout scheduled in 2s; open video fullscreen now.".into();
                    this.scheduled=Some(cx.spawn(async move |this,cx| {
                        cx.background_executor().timer(Duration::from_secs(2)).await;
                        this.update(cx,|this,cx|{
                            this.session.update(cx,|session,cx|{session.snapshot=None;cx.notify();});
                            this.status="Mock logout completed; playback, metadata and fullscreen are cleared.".into();cx.notify();
                        }).ok();
                    }));cx.notify();
                })))
                .child(Button::new("fixture-restore",theme).variant(ButtonVariant::Secondary).label("Restore mock login").on_click(cx.listener(|this,_,_,cx|{this.session.update(cx,|session,cx|{session.snapshot=Some(snapshot());cx.notify();});this.status="Mock account restored; no saved credentials read.".into();cx.notify();})))
                .child(Button::new("fixture-repair",theme).variant(ButtonVariant::Secondary).label("Repair retry URL").on_click(cx.listener(|this,_,_,cx|{this.server.repair.store(true,Ordering::Release);this.status="Retry URL now serves real MP4; click its Retry control.".into();cx.notify();}))))
            .child(self.status.clone());
        for (key, path, kind) in [
            ("audio", "/silent.mp3", "Audio"),
            ("video", "/silent.mp4", "Video"),
            ("retry", "/recover.mp4", "Video"),
        ] {
            let file = self.file(key, path, kind);
            if let Some(player) =
                media_player::render_attachment(&self.players, key, &file, width, cx)
            {
                body = body.child(player);
            }
        }
        body
    }
}
fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            fonts::register(cx);
            let light = std::env::args().any(|arg| arg == "--light");
            cx.set_global(CurrentTheme(if light {
                Theme::light()
            } else {
                Theme::dark()
            }));
            gpui_kit::component::theme::Theme::change(
                if light {
                    gpui_kit::component::theme::ThemeMode::Light
                } else {
                    gpui_kit::component::theme::ThemeMode::Dark
                },
                None,
                cx,
            );
            let compact = std::env::args().any(|arg| arg == "--compact");
            gpui_kit::open_window(
                WindowOptions {
                    app_id: Some("stonks-inline-media-preview".into()),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Offline inline media".into()),
                        ..Default::default()
                    }),
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        if compact {
                            size(px(560.), px(700.))
                        } else {
                            size(px(1000.), px(820.))
                        },
                        cx,
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| Preview::new(window, cx)),
            )
            .expect("offline media window");
            cx.activate(true);
        });
}
