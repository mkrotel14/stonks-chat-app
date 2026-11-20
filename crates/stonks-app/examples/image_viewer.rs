//! Offline fixture using the production viewer/cache. No saved account is read.
//! --compact opens at 500x400; --light changes the underlying page only.
//! All image requests go to a synthetic, credential-free loopback HTTP server.
#![allow(dead_code)]
use gpui_kit::*;
use std::{
    collections::HashMap,
    io::{Cursor, Read, Write},
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
use stonks_ui::{components::Button, fonts, theme::CurrentTheme, Theme};
#[path = "../src/features/conversation/media/image_viewer/mod.rs"]
mod image_viewer;
#[path = "../src/shared/mod.rs"]
mod shared;
#[path = "../src/runtime.rs"]
mod runtime;
#[path = "../src/session/mod.rs"]
mod session;
use image_viewer::{ImageViewer, ImageViewerEvent, ViewerImage};
use shared::images::Images;
use session::Session;

struct LocalSelection(ServerSelection);
impl SessionAccess for LocalSelection {
    fn server_selection(&self) -> Result<Option<ServerSelection>, ContextError> {
        Ok(Some(self.0.clone()))
    }
    fn token(&self, _: &ServerSelection) -> Result<Option<String>, ContextError> {
        panic!("offline media must never request credentials")
    }
}
struct LocalServer {
    base: String,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl LocalServer {
    fn new() -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let base = format!("http://{}", listener.local_addr()?);
        let png = |width, height| {
            let pixels = grid(width, height, 0);
            let mut out = Cursor::new(Vec::new());
            pixels
                .write_to(&mut out, image::ImageFormat::Png)
                .expect("synthetic PNG");
            out.into_inner()
        };
        let mut gif = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut gif);
            encoder
                .set_repeat(image::codecs::gif::Repeat::Infinite)
                .expect("GIF repeat");
            for phase in 0..8 {
                encoder
                    .encode_frame(image::Frame::from_parts(
                        grid(320, 180, phase * 32),
                        0,
                        0,
                        image::Delay::from_numer_denom_ms(120, 1),
                    ))
                    .expect("synthetic GIF frame");
            }
        }
        let routes = Arc::new(HashMap::from([
            ("/wide.png", (png(1600, 900), "image/png")),
            ("/portrait.png", (png(600, 1600), "image/png")),
            ("/small.png", (png(120, 80), "image/png")),
            ("/preview.png", (png(320, 180), "image/png")),
            ("/slow.png", (png(1600, 900), "image/png")),
            ("/warm.png", (png(1600, 900), "image/png")),
            ("/animated.gif", (gif, "image/gif")),
        ]));
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let worker = thread::spawn(move || {
            let mut clients = Vec::new();
            while !stopping.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        let routes = routes.clone();
                        let stopping = stopping.clone();
                        clients.push(thread::spawn(move || {
                            let _ = socket.set_read_timeout(Some(Duration::from_secs(2)));
                            let _ = socket.set_write_timeout(Some(Duration::from_secs(2)));
                            let mut request = [0; 4096];
                            let Ok(length) = socket.read(&mut request) else { return };
                            let request = String::from_utf8_lossy(&request[..length]);
                            let path = request.split_whitespace().nth(1).unwrap_or("");
                            if matches!(path, "/slow.png" | "/warm.png") {
                                for _ in 0..30 {
                                    if stopping.load(Ordering::Relaxed) { return }
                                    thread::sleep(Duration::from_millis(50));
                                }
                            }
                            if let Some((bytes, kind)) = routes.get(path) {
                                let _ = write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len());
                                let _ = socket.write_all(bytes);
                            } else {
                                let _ = socket.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                            }
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => break,
                }
                let mut index = 0;
                while index < clients.len() {
                    if clients[index].is_finished() {
                        let _ = clients.swap_remove(index).join();
                    } else {
                        index += 1;
                    }
                }
            }
            for client in clients {
                let _ = client.join();
            }
        });
        Ok(Self {
            base,
            stop,
            worker: Some(worker),
        })
    }
}
impl Drop for LocalServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn grid(width: u32, height: u32, phase: u32) -> image::RgbaImage {
    image::RgbaImage::from_fn(width, height, |x, y| {
        image::Rgba(if (x + phase) % 80 < 3 || y % 80 < 3 {
            [255, 255, 255, 255]
        } else if (x + phase) % 320 < 160 {
            [220, 40, 40, 255]
        } else {
            [40, 90, 230, 255]
        })
    })
}
struct Preview {
    viewer: Entity<ImageViewer>,
    images: Entity<Images>,
    _session: Entity<Session>,
    server: LocalServer,
    status: String,
    _subscriptions: Vec<Subscription>,
}
impl Preview {
    fn new(cx: &mut Context<Self>) -> Self {
        let server = LocalServer::new().expect("offline loopback image server");
        let selection = ServerSelection {
            deployment_url: server.base.clone(),
            display_name: "Offline fixture".into(),
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
                .expect("offline HTTP client"),
        );
        let session = cx.new(|_| {
            let mut session = Session::new(policy.clone());
            session.server = Some(selection);
            session.snapshot = Some(AppSnapshot {
                current_user: serde_json::from_value(
                    serde_json::json!({"_id":"fixture-user", "username":"Offline"}),
                )
                .expect("fixture user"),
                users: vec![],
                servers: vec![],
                channels: vec![],
                members: vec![],
                unreads: vec![],
                voice_states: vec![],
            });
            session
        });
        let images = cx.new(|cx| Images::new(policy, &session, cx));
        images.update(cx, |images, _| {
            images.set_sources(
                "viewer-fixture",
                [(format!("{}/preview.png", server.base), 1280)],
            )
        });
        let viewer = cx.new(|cx| ImageViewer::new(images.clone(), cx));
        let events = cx.subscribe(&viewer, |this, _, event, cx| {
            this.status = match event {
                ImageViewerEvent::Close => "Closed; focus returned to opener".into(),
                ImageViewerEvent::OpenExternal(url) => format!("Intercepted open original: {url}"),
            };
            cx.notify();
        });
        let observation = cx.observe(&images, |_, _, cx| cx.notify());
        Self {
            viewer,
            images,
            _session: session,
            server,
            status: "All data is synthetic; no account or saved preferences are accessed.".into(),
            _subscriptions: vec![events, observation],
        }
    }
}
impl Render for Preview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(cx);
        let preview_url = format!("{}/preview.png", self.server.base);
        let thumbnail = self
            .images
            .update(cx, |images, cx| images.get(Some(&preview_url), 1280, cx));
        let mut page = div().relative().size_full().font_family(fonts::FAMILY).bg(theme.canvas).text_color(theme.text)
            .child(div().p(px(24.)).flex().flex_col().gap(px(16.)).child("Production image viewer — offline fixture")
                .child("Fit, then zoom to 400% and scroll both axes. Tab cycles toolbar; Escape/outside closes.")
                .child(div().flex().flex_wrap().gap(px(8.)).children([
                    ("wide.png", "Landscape"), ("portrait.png", "Portrait"), ("small.png", "Small / no upscale"),
                    ("animated.gif", "Animated GIF"), ("slow.png", "Loading"), ("failure.png", "Failure / retry"), ("warm.png", "Warm preview"), ("failed-original.png", "Preview / failed original")
                ].into_iter().enumerate().map(|(index,(path,label))| {
                    let url = format!("{}/{path}", self.server.base);
                    let preview = preview_url.clone();
                    Button::new(("viewer-case",index),theme).label(label).on_click(cx.listener(move |this,_,window,cx| {
                        let mut target = ViewerImage::new(url.clone(),path);
                        if matches!(path, "warm.png" | "failed-original.png") { target = target.dimensions(1600,900).preview(preview.clone()); }
                        this.viewer.update(cx, |viewer,cx| viewer.open(target,window,cx));
                    }))
                })))
                .child(self.status.clone()));
        if let Some(thumbnail) = thumbnail {
            page = page.child(
                div()
                    .absolute()
                    .left(px(24.))
                    .bottom(px(24.))
                    .child(img(thumbnail).w(px(160.)).h(px(90.))),
            );
        }
        page.child(self.viewer.clone())
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
            let compact = std::env::args().any(|arg| arg == "--compact");
            gpui_kit::open_window(
                WindowOptions {
                    app_id: Some("stonks-image-viewer-preview".into()),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Offline image viewer".into()),
                        ..Default::default()
                    }),
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        if compact {
                            size(px(500.), px(400.))
                        } else {
                            size(px(1200.), px(800.))
                        },
                        cx,
                    ))),
                    ..Default::default()
                },
                cx,
                |_, cx| cx.new(Preview::new),
            )
            .expect("offline viewer window");
            cx.activate(true);
        });
}
