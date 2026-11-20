//! Native presentation only; the caller owns the LiveKit subscription and audio.
use gpui_kit::{
    component::slider::{SliderEvent, SliderState, SliderValue},
    prelude::FluentBuilder,
    *,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use stonks_ui::{
    components::{Button, ButtonVariant, UiCloseButton, UiRange},
    i18n::t,
    icons::Icon,
    Theme,
};
use tokio::sync::watch;

/// A per-track latest-frame feed. Move or clone it into the decoder task.
/// Dropping the last producer closes the viewer; closing the viewer stops the feed.
#[derive(Clone)]
pub(crate) struct FrameFeed(watch::Sender<Option<Arc<RenderImage>>>);

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FeedError {
    InvalidFrame,
    ViewerClosed,
}

impl FrameFeed {
    pub(super) fn channel() -> (Self, watch::Receiver<Option<Arc<RenderImage>>>) {
        let (tx, rx) = watch::channel(None);
        (Self(tx), rx)
    }

    /// Submit an owned, tightly packed, top-down 8-bit BGRA buffer. Strip row
    /// padding and convert from LiveKit's decoded format on the producer side.
    /// Never waits for the UI: a newer frame replaces any unread frame.
    pub(crate) fn push_bgra(
        &self,
        width: u32,
        height: u32,
        bytes: Vec<u8>,
    ) -> std::result::Result<(), FeedError> {
        if self.0.is_closed() {
            return Err(FeedError::ViewerClosed);
        }
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4));
        if width == 0 || height == 0 || expected != Some(bytes.len()) {
            return Err(FeedError::InvalidFrame);
        }
        // GPUI's RenderImage upload expects BGRA, despite image's container name.
        let pixels =
            image::RgbaImage::from_raw(width, height, bytes).ok_or(FeedError::InvalidFrame)?;
        self.0
            .send(Some(Arc::new(RenderImage::new(vec![image::Frame::new(
                pixels,
            )]))))
            .map_err(|_| FeedError::ViewerClosed)
    }

    pub(super) fn clear(&self) {
        let _ = self.0.send(None);
    }

    /// Resolves when the viewer has shut down, even if no frames are arriving.
    pub(crate) async fn closed(&self) {
        self.0.closed().await;
    }
}

type SetGain = Box<dyn Fn(u8, &mut App)>;

pub(super) struct AudioControls {
    present: watch::Receiver<bool>,
    gain: u8,
    set_gain: SetGain,
}

impl AudioControls {
    pub(super) fn new(
        present: watch::Receiver<bool>,
        gain: u8,
        set_gain: impl Fn(u8, &mut App) + 'static,
    ) -> Self {
        Self {
            present,
            gain,
            set_gain: Box::new(set_gain),
        }
    }
}

struct AudioRail {
    present: bool,
    gain: u8,
    previous: u8,
    slider: Entity<SliderState>,
    set_gain: SetGain,
    _changes: Subscription,
    _presence: Task<()>,
}

impl AudioRail {
    fn new(config: AudioControls, window: &mut Window, cx: &mut Context<Viewer>) -> Self {
        let AudioControls {
            mut present,
            gain,
            set_gain,
        } = config;
        let initial = *present.borrow_and_update();
        let slider = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(100.)
                .step(1.)
                .default_value(f32::from(gain))
        });
        let changes = cx.subscribe(&slider, |viewer, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(SliderValue::Single(value)) = event {
                if let Some(audio) = viewer.audio.as_mut().filter(|audio| audio.present) {
                    audio.update_gain(value.round() as u8, cx);
                }
            }
        });
        let presence = cx.spawn_in(window, async move |viewer, cx| {
            while present.changed().await.is_ok() {
                let present = *present.borrow_and_update();
                let _ = cx.update(|_, cx| {
                    viewer.update(cx, |viewer, cx| {
                        if let Some(audio) = &mut viewer.audio {
                            audio.present = present;
                        }
                        cx.notify();
                    })
                });
            }
        });
        Self {
            present: initial,
            gain,
            previous: if gain == 0 { 80 } else { gain },
            slider,
            set_gain,
            _changes: changes,
            _presence: presence,
        }
    }

    fn update_gain(&mut self, gain: u8, cx: &mut Context<Viewer>) {
        if self.gain == gain {
            return;
        }
        self.gain = gain;
        if gain != 0 {
            self.previous = gain;
        }
        (self.set_gain)(gain, cx);
        cx.notify();
    }
}

/// Open one native top-level viewer on the GPUI thread. Pass the opener's
/// display to center it there; subsequent placement is compositor-managed.
/// Keep the handle for explicit close/activation. `on_close` runs once on
/// GPUI after the view is released (not if window creation fails).
pub(crate) fn open(
    title: SharedString,
    display_id: Option<DisplayId>,
    audio: Option<AudioControls>,
    on_close: impl FnOnce(&mut App) + 'static,
    cx: &mut App,
) -> gpui_kit::Result<(AnyWindowHandle, FrameFeed)> {
    let (feed, rx) = FrameFeed::channel();
    let bounds = Bounds::centered(display_id, size(px(720.), px(450.)), cx);
    let (handle, _) = gpui_kit::open_window(
        WindowOptions {
            app_id: Some("stonks-chat-gpui".into()),
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(title.clone()),
                appears_transparent: true,
                ..Default::default()
            }),
            display_id,
            window_decorations: Some(WindowDecorations::Client),
            window_min_size: Some(size(px(360.), px(240.))),
            is_movable: true,
            is_resizable: true,
            ..Default::default()
        },
        cx,
        move |window, cx| cx.new(|cx| Viewer::new(title, rx, audio, on_close, window, cx)),
    )?;
    Ok((handle, feed))
}

const CHROME_TIMEOUT: Duration = Duration::from_millis(2600);

struct Viewer {
    title: SharedString,
    audio: Option<AudioRail>,
    focus: FocusHandle,
    rail_focus: FocusHandle,
    frame: Option<Arc<RenderImage>>,
    last_activity: Instant,
    chrome_visible: bool,
    _frames: Task<()>,
    _chrome: Task<()>,
    _bounds: Subscription,
}

impl Viewer {
    fn new(
        title: SharedString,
        mut rx: watch::Receiver<Option<Arc<RenderImage>>>,
        audio: Option<AudioControls>,
        on_close: impl FnOnce(&mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        cx.on_release(move |_, cx| cx.defer(on_close)).detach();
        let frames = cx.spawn_in(window, async move |this, cx| {
            while rx.changed().await.is_ok() {
                let frame = rx.borrow_and_update().clone();
                if cx
                    .update(|window, cx| {
                        this.update(cx, |viewer, cx| {
                            if let Some(old) = std::mem::replace(&mut viewer.frame, frame) {
                                let _ = window.drop_image(old);
                            }
                            cx.notify();
                        })
                    })
                    .ok()
                    .and_then(|result| result.ok())
                    .is_none()
                {
                    return;
                }
            }
            let _ = cx.update(|window, _| window.remove_window());
        });
        let chrome = cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(200))
                .await;
            if cx
                .update(|window, cx| {
                    this.update(cx, |viewer, cx| {
                        if viewer.chrome_visible
                            && viewer.frame.is_some()
                            && viewer.last_activity.elapsed() >= CHROME_TIMEOUT
                            && !viewer.rail_focus.contains_focused(window, cx)
                        {
                            viewer.chrome_visible = false;
                            cx.notify();
                        }
                    })
                })
                .ok()
                .and_then(|result| result.ok())
                .is_none()
            {
                break;
            }
        });
        Self {
            title,
            audio: audio.map(|audio| AudioRail::new(audio, window, cx)),
            focus,
            rail_focus: cx.focus_handle(),
            frame: None,
            last_activity: Instant::now(),
            chrome_visible: true,
            _frames: frames,
            _chrome: chrome,
            _bounds: cx.observe_window_bounds(window, |_, _, cx| cx.notify()),
        }
    }

    fn reveal(&mut self, cx: &mut Context<Self>) {
        self.last_activity = Instant::now();
        if !self.chrome_visible {
            self.chrome_visible = true;
            cx.notify();
        }
    }
}

impl Render for Viewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::dark();
        let fullscreen = window.is_fullscreen();
        let (icon, label) = if fullscreen {
            (Icon::Minimize, t("media.exitFullscreen"))
        } else {
            (Icon::Maximize, t("media.fullscreen"))
        };
        let show_chrome = self.chrome_visible;
        let toggle = |_: &ClickEvent, window: &mut Window, _: &mut App| {
            window.toggle_fullscreen();
            window.refresh();
        };
        div()
            .id("screen-viewer")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .bg(theme.media_surface)
            .text_color(theme.text)
            .font_family(stonks_ui::fonts::FAMILY)
            .overflow_hidden()
            .on_mouse_move(cx.listener(|viewer, _, _, cx| viewer.reveal(cx)))
            .capture_key_down(cx.listener(|viewer, event: &KeyDownEvent, window, cx| {
                viewer.reveal(cx);
                match event.keystroke.key.as_str() {
                    "escape" if window.is_fullscreen() => window.toggle_fullscreen(),
                    "f11" if !event.is_held => window.toggle_fullscreen(),
                    _ => return,
                }
                window.prevent_default();
                cx.stop_propagation();
                cx.notify();
            }))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .on_mouse_down(MouseButton::Left, |_, window, _| {
                        if !window.is_fullscreen() {
                            window.start_window_move();
                        }
                    })
                    .when_some(self.frame.clone(), |stage, frame| {
                        stage.child(
                            img(frame)
                                .id("screen-viewer-frame")
                                .size_full()
                                .object_fit(ObjectFit::Contain),
                        )
                    })
                    .when(self.frame.is_none(), |stage| {
                        stage.child(
                            div()
                                .rounded(px(theme.radius_pill))
                                .bg(theme.media_chrome)
                                .text_color(theme.text_muted)
                                .text_sm()
                                .px(px(11.))
                                .py(px(8.))
                                .child("Waiting for the native video stream…"),
                        )
                    }),
            )
            .when(show_chrome, |root| {
                root.child(
                    div()
                        .absolute()
                        .top(px(12.))
                        .left(px(12.))
                        .right(px(12.))
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .min_w_0()
                                .max_w_full()
                                .gap(px(8.))
                                .px(px(11.))
                                .py(px(7.))
                                .border_1()
                                .border_color(theme.outline)
                                .rounded(px(theme.radius_pill))
                                .bg(theme.media_chrome)
                                .cursor_grab()
                                .on_mouse_down(MouseButton::Left, |_, window, _| {
                                    if !window.is_fullscreen() {
                                        window.start_window_move();
                                    }
                                })
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .text_xs()
                                        .font_weight(FontWeight::BLACK)
                                        .text_color(theme.success_text)
                                        .child("LIVE"),
                                )
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(self.title.clone()),
                                ),
                        ),
                )
                .child(
                    div()
                        .absolute()
                        .bottom(px(if window.viewport_size().height < px(280.) {
                            8.
                        } else {
                            12.
                        }))
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .id("screen-viewer-controls")
                                .track_focus(&self.rail_focus)
                                .occlude()
                                .flex()
                                .items_center()
                                .gap(px(7.))
                                .px(px(9.))
                                .py(px(8.))
                                .border_1()
                                .border_color(theme.outline)
                                .rounded(px(theme.radius_pill))
                                .bg(theme.media_chrome_raised)
                                .when_some(
                                    self.audio.as_ref().filter(|audio| audio.present),
                                    |rail, audio| {
                                        let muted = audio.gain == 0;
                                        rail.child(
                                            icon_button(
                                                "screen-viewer-mute",
                                                if muted { Icon::VolumeX } else { Icon::Volume2 },
                                                if muted { "Unmute" } else { "Mute" }.into(),
                                                theme,
                                            )
                                            .accessibility_label(if muted {
                                                "Unmute share audio"
                                            } else {
                                                "Mute share audio"
                                            })
                                            .on_click(
                                                cx.listener(|viewer, _, window, cx| {
                                                    if let Some(audio) = viewer
                                                        .audio
                                                        .as_mut()
                                                        .filter(|audio| audio.present)
                                                    {
                                                        let gain = if audio.gain == 0 {
                                                            audio.previous
                                                        } else {
                                                            0
                                                        };
                                                        audio.update_gain(gain, cx);
                                                        audio.slider.update(cx, |slider, cx| {
                                                            slider.set_value(
                                                                f32::from(gain),
                                                                window,
                                                                cx,
                                                            )
                                                        });
                                                    }
                                                }),
                                            ),
                                        )
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap(px(6.))
                                                .w(px(
                                                    if window.viewport_size().width <= px(460.) {
                                                        (f32::from(window.viewport_size().width)
                                                            * 0.31)
                                                            .clamp(82., 130.)
                                                    } else {
                                                        (f32::from(window.viewport_size().width)
                                                            * 0.36)
                                                            .min(220.)
                                                    },
                                                ))
                                                .child(
                                                    div().flex_1().min_w_0().child(
                                                        UiRange::new(&audio.slider, theme)
                                                            .label("Share audio volume")
                                                            .slim(true),
                                                    ),
                                                )
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(theme.text_muted)
                                                        .child(format!("{}%", audio.gain)),
                                                ),
                                        )
                                        .child(div().w(px(1.)).h(px(22.)).bg(theme.outline))
                                    },
                                )
                                .when(!fullscreen && window.window_controls().minimize, |rail| {
                                    rail.child(
                                        icon_button(
                                            "screen-viewer-minimize",
                                            Icon::Minus,
                                            "Minimize".into(),
                                            theme,
                                        )
                                        .on_click(|_, window, _| window.minimize_window()),
                                    )
                                })
                                .child(
                                    icon_button(
                                        "screen-viewer-fullscreen",
                                        icon,
                                        label.into(),
                                        theme,
                                    )
                                    .on_click(toggle),
                                )
                                .child(
                                    UiCloseButton::new("screen-viewer-close", theme)
                                        .on_click(|_, window, _| window.remove_window()),
                                ),
                        ),
                )
            })
            .when(!fullscreen && !window.is_maximized(), |root| {
                root.children(resize_edges())
            })
    }
}

fn icon_button(id: &'static str, icon: Icon, label: SharedString, theme: Theme) -> Button {
    Button::new(id, theme)
        .variant(ButtonVariant::IconSmall)
        .icon_only(true)
        .tooltip(label)
        .leading(icon.render_stroke(theme.text, 15., 2.))
}

fn resize_edges() -> impl Iterator<Item = Div> {
    // Explicit client-side grips: native X11/Wayland resize, not app geometry.
    use ResizeEdge::*;
    [
        Top,
        TopRight,
        Right,
        BottomRight,
        Bottom,
        BottomLeft,
        Left,
        TopLeft,
    ]
    .into_iter()
    .map(|edge| {
        let grip = div().absolute().occlude();
        let n = px(6.);
        let grip = match edge {
            Top => grip.top_0().left(n).right(n).h(n).cursor_ns_resize(),
            Bottom => grip.bottom_0().left(n).right(n).h(n).cursor_ns_resize(),
            Left => grip.left_0().top(n).bottom(n).w(n).cursor_ew_resize(),
            Right => grip.right_0().top(n).bottom(n).w(n).cursor_ew_resize(),
            TopLeft => grip.top_0().left_0().size(n).cursor_nwse_resize(),
            TopRight => grip.top_0().right_0().size(n).cursor_nesw_resize(),
            BottomLeft => grip.bottom_0().left_0().size(n).cursor_nesw_resize(),
            BottomRight => grip.bottom_0().right_0().size(n).cursor_nwse_resize(),
        };
        grip.on_mouse_down(MouseButton::Left, move |_, window, cx| {
            window.start_window_resize(edge);
            cx.stop_propagation();
        })
    })
}

#[cfg(test)]
mod tests {
    use super::{watch, FeedError, FrameFeed};

    #[test]
    fn feed_keeps_only_latest_frame_with_bgra_bytes_intact() {
        let (tx, mut rx) = watch::channel(None);
        let feed = FrameFeed(tx);
        feed.push_bgra(1, 1, vec![0, 0, 255, 255]).unwrap();
        let latest = vec![255, 0, 0, 255, 0, 255, 0, 255];
        feed.push_bgra(2, 1, latest.clone()).unwrap();
        let frame = rx.borrow_and_update();
        assert_eq!(frame.as_ref().unwrap().as_bytes(0), Some(latest.as_slice()));
        drop(frame);
        assert!(!rx.has_changed().unwrap());
    }

    #[test]
    fn invalid_frames_do_not_publish() {
        let (tx, rx) = watch::channel(None);
        let feed = FrameFeed(tx);
        for (width, height, len) in [
            (0, 1, 0),
            (1, 0, 0),
            (2, 1, 7),
            (1, 1, 5),
            (u32::MAX, u32::MAX, 0),
        ] {
            assert_eq!(
                feed.push_bgra(width, height, vec![0; len]),
                Err(FeedError::InvalidFrame)
            );
        }
        assert!(!rx.has_changed().unwrap());
    }

    #[test]
    fn feed_closure_works_in_both_directions() {
        let (tx, rx) = watch::channel(None);
        let feed = FrameFeed(tx);
        let worker = feed.clone();
        drop(feed);
        assert!(!rx.has_changed().unwrap());
        drop(worker);
        assert!(rx.has_changed().is_err());

        let (tx, rx) = watch::channel(None);
        let feed = FrameFeed(tx);
        drop(rx);
        assert_eq!(
            feed.push_bgra(1, 1, vec![0; 4]),
            Err(FeedError::ViewerClosed)
        );
        assert!(futures_util::FutureExt::now_or_never(feed.closed()).is_some());
    }
}
