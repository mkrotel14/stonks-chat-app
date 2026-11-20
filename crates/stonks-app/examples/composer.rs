//! Offline actual composer/picker fixture. No saved account or gateway.
//! Synthetic emoji API/media are served on loopback; all mutation methods are rejected.
#![allow(dead_code)]
use gpui_kit::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
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
#[path = "../src/features/conversation/composer/mod.rs"]
mod composer;
#[path = "../src/shared/mod.rs"]
mod shared;
#[path = "../src/runtime.rs"]
mod runtime;
#[path = "../src/session/mod.rs"]
pub mod session;
use composer::Composer;
use shared::images::Images;
use session::Session;
const CHANNEL: &str = "01M00000000000000000000001";
const SERVER: &str = "01M00000000000000000000002";
struct MockSession(ServerSelection);
impl SessionAccess for MockSession {
    fn server_selection(&self) -> Result<Option<ServerSelection>, ContextError> {
        Ok(Some(self.0.clone()))
    }
    fn token(&self, _: &ServerSelection) -> Result<Option<String>, ContextError> {
        Ok(Some("offline-fixture-only".into()))
    }
}
struct LocalServer {
    base: String,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl LocalServer {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let shutdown = stop.clone();
        let worker = thread::spawn(move || {
            let mut encoded = std::io::Cursor::new(Vec::new());
            image::RgbaImage::from_pixel(24, 24, image::Rgba([180, 100, 220, 255]))
                .write_to(&mut encoded, image::ImageFormat::Png)
                .unwrap();
            while !shutdown.load(Ordering::Acquire) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                let mut buffer = [0; 8192];
                let count = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..count]);
                let path = request.split_whitespace().nth(1).unwrap_or("");
                let (status, kind, bytes) = if !request.starts_with("GET ") {
                    (
                        "405 Method Not Allowed",
                        "text/plain",
                        b"Offline fixture rejects all mutation requests".to_vec(),
                    )
                } else if path.ends_with("/emojis") {
                    ("200 OK","application/json",br#"[{"_id":"01M00000000000000000000003","name":"wave"},{"_id":"01M00000000000000000000004","name":"purple"}]"#.to_vec())
                } else if path.starts_with("/emojis/") {
                    ("200 OK", "image/png", encoded.get_ref().clone())
                } else {
                    ("404 Not Found", "text/plain", Vec::new())
                };
                let header=format!("HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",bytes.len());
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&bytes);
            }
        });
        Self {
            base,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for LocalServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn snapshot() -> AppSnapshot {
    AppSnapshot {
    current_user:serde_json::from_value(serde_json::json!({"_id":"01M00000000000000000000005","username":"Offline user"})).unwrap(),
    users:Vec::new(),members:Vec::new(),unreads:Vec::new(),voice_states:Vec::new(),
    servers:vec![serde_json::from_value(serde_json::json!({"_id":SERVER,"name":"Offline server","owner":"01M00000000000000000000005","channels":[CHANNEL]})).unwrap()],
    channels:vec![serde_json::from_value(serde_json::json!({"_id":CHANNEL,"channel_type":"TextChannel","server":SERVER,"name":"General"})).unwrap()],
}
}
struct Preview {
    composer: Entity<Composer>,
    session: Entity<Session>,
    editor: Entity<gpui_kit::component::input::TextareaState>,
    _server: LocalServer,
    status: String,
    channel: bool,
}
impl Preview {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let server = LocalServer::new();
        let selection = ServerSelection {
            deployment_url: server.base.clone(),
            display_name: "Offline composer".into(),
            official: false,
            api_version: "fixture".into(),
            capabilities: ServerCapabilities::default(),
            endpoints: ServerEndpoints {
                api: server.base.clone(),
                websocket: "ws://127.0.0.1/unused".into(),
                autumn: Some(server.base.clone()),
                january: None,
                gifbox: None,
                livekit_enabled: false,
            },
        };
        let policy = Arc::new(HttpPolicy::new(Arc::new(MockSession(selection.clone()))).unwrap());
        let session = cx.new(|_| {
            let mut session = Session::new(policy.clone());
            session.server = Some(selection);
            session.snapshot = Some(snapshot());
            session
        });
        let images = cx.new(|cx| Images::new(policy.clone(), &session, cx));
        let composer = cx.new(|cx| Composer::new(policy, window, cx));
        composer.update(cx, |composer, cx| {
            composer.set_channel(CHANNEL.into(), "General", window, cx);
            composer.set_emoji_context(session.clone(), images, cx);
        });
        let editor = composer.read(cx).fixture_editor();
        let original = "Olá 🌍 selected tail";
        let start = "Olá 🌍 ".len();
        editor.update(cx, |input, cx| {
            input.set_value(original, window, cx);
            input.set_selected_range(start..start + "selected".len(), cx);
        });
        composer.update(cx, |composer, cx| {
            composer.insert_text("👨‍👩‍👧‍👦".into(), window, cx)
        });
        assert_eq!(editor.read(cx).value().as_ref(), "Olá 🌍 👨‍👩‍👧‍👦 tail");
        let caret = start + "👨‍👩‍👧‍👦".len();
        assert_eq!(editor.read(cx).selected_range(), caret..caret);
        composer.update(cx, |composer, cx| {
            composer.insert_text("👍🏽".into(), window, cx)
        });
        assert_eq!(editor.read(cx).value().as_ref(), "Olá 🌍 👨‍👩‍👧‍👦👍🏽 tail");
        let mut preview=Self{composer,session,editor,_server:server,status:"PASS: actual native selection replacement/cursor insertion preserves Unicode. Pick any emoji to replace selected text.".into(),channel:false};
        preview.seed(window, cx);
        preview
    }
    fn seed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |input, cx| {
            input.select_all(window, cx);
        });
        self.composer.update(cx, |composer, cx| {
            composer.insert_text("Olá 🌍 selected tail".into(), window, cx)
        });
        let start = "Olá 🌍 ".len();
        self.editor.update(cx, |input, cx| {
            input.set_selected_range(start..start + "selected".len(), cx);
            input.focus(window, cx);
        });
        cx.notify();
    }
}
impl Render for Preview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(cx);
        div().size_full().font_family(fonts::FAMILY).bg(theme.canvas).text_color(theme.text).flex().flex_col().p(px(16.)).gap(px(12.))
        .child("Offline production composer — source emoji picker")
        .child(self.status.clone())
        .child(div().flex().flex_wrap().gap(px(8.))
            .child(Button::new("fixture-seed",theme).variant(ButtonVariant::Secondary).label("Seed selected text").on_click(cx.listener(|this,_,window,cx|this.seed(window,cx))))
            .child(Button::new("fixture-channel",theme).variant(ButtonVariant::Secondary).label("Switch channel").on_click(cx.listener(|this,_,window,cx|{this.channel=!this.channel;let id=if this.channel{"01M00000000000000000000006"}else{CHANNEL};this.composer.update(cx,|composer,cx|composer.set_channel(id.into(),"General",window,cx));cx.notify();})))
            .child(Button::new("fixture-logout",theme).variant(ButtonVariant::Secondary).label("Mock logout").on_click(cx.listener(|this,_,window,cx|{this.session.update(cx,|session,cx|{session.snapshot=None;cx.notify();});this.composer.update(cx,|composer,cx|composer.reset(window,cx));cx.notify();}))))
        .child(div().flex_1().min_h_0().relative().rounded(px(32.)).bg(theme.surface).overflow_hidden()
            .child(div().p(px(24.)).child("Emoji picker: click smile; search 'brazil', 'thumbsup' or 'skin tone'; choose category; Show more; Esc/outside dismiss. Keyboard Tab → smile → Enter opens. Draft text is selected initially. Custom emoji below uses loopback PNGs. The bar floats over this content."))
            .child(div().absolute().left(px(8.)).right(px(8.)).bottom(px(8.)).child(self.composer.clone())))
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
                    app_id: Some("stonks-composer-preview".into()),
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        if compact {
                            size(px(560.), px(650.))
                        } else {
                            size(px(900.), px(760.))
                        },
                        cx,
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| Preview::new(window, cx)),
            )
            .expect("offline composer window");
            cx.activate(true);
        });
}
