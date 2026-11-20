//! Attachment fullscreen owns a presentation window, not another media load.
//! Its frame, clock, seek, volume and controls all address the existing player.
use super::*;
use gpui_kit::base::FocusTrapElement;
use gpui_kit::prelude::FluentBuilder;
use std::{cell::Cell, rc::Rc};

pub(super) struct FullscreenWindow {
    handle: AnyWindowHandle,
    restore: Rc<Cell<bool>>,
}
pub(super) struct VideoFullscreen {
    player: WeakEntity<MediaPlayer>,
    focus: FocusHandle,
    _player: Option<Subscription>,
}
pub(super) fn release_window(player: &mut MediaPlayer, cx: &mut App) {
    if player.fullscreen_opening || player.fullscreen.is_some() {
        player.fullscreen_opening = false;
        player.fullscreen_generation = player.fullscreen_generation.wrapping_add(1);
    }
    if let Some(owner) = player.fullscreen.take() {
        owner.restore.set(false);
        cx.defer(move |cx| {
            let _ = owner
                .handle
                .update(cx, |_, window, _| window.remove_window());
        });
    }
}
fn create_window(
    player: WeakEntity<MediaPlayer>,
    generation: u64,
    opener: AnyWindowHandle,
    previous: Option<WeakFocusHandle>,
    bounds: Bounds<Pixels>,
    display_id: Option<DisplayId>,
    cx: &mut App,
) {
    let Some(owner) = player.upgrade() else {
        return;
    };
    let valid = {
        let state = owner.read(cx);
        state.enabled
            && state.fullscreen_generation == generation
            && state.fullscreen_opening
            && state.current.frame.is_some()
    };
    if !valid {
        return;
    }
    let restore = Rc::new(Cell::new(true));
    let restore_on_release = restore.clone();
    let opened = gpui_kit::open_window(
        WindowOptions {
            app_id: Some("stonks-chat-gpui".into()),
            window_bounds: Some(WindowBounds::Fullscreen(bounds)),
            display_id,
            window_decorations: Some(WindowDecorations::Client),
            titlebar: None,
            is_movable: false,
            is_minimizable: false,
            ..Default::default()
        },
        cx,
        move |window, cx| {
            cx.new(|cx| {
                let focus = cx.focus_handle();
                focus.focus(window, cx);
                let view_window = window.window_handle();
                let observation = player
                    .upgrade()
                    .map(|player| cx.observe(&player, |_, _, cx| cx.notify()));
                let release_player = player.clone();
                cx.on_release(move |_, cx| {
                    let should_restore = restore_on_release.get();
                    cx.defer(move |cx| {
                        let current = release_player
                            .update(cx, |player, cx| {
                                if player.fullscreen.as_ref().is_some_and(|owner| {
                                    owner.handle.window_id() == view_window.window_id()
                                }) {
                                    player.fullscreen = None;
                                    cx.notify();
                                }
                                player.fullscreen_generation == generation
                            })
                            .unwrap_or(false);
                        if should_restore && current {
                            let _ = opener.update(cx, |_, window, cx| {
                                window.activate_window();
                                if let Some(focus) = previous.and_then(|focus| focus.upgrade()) {
                                    focus.focus(window, cx);
                                }
                            });
                        }
                    });
                })
                .detach();
                VideoFullscreen {
                    player,
                    focus,
                    _player: observation,
                }
            })
        },
    );
    match opened {
        Ok((handle, _)) => {
            let installed = owner.update(cx, |player, cx| {
                if !player.enabled
                    || player.fullscreen_generation != generation
                    || !player.fullscreen_opening
                {
                    return false;
                }
                player.fullscreen_opening = false;
                player.fullscreen = Some(FullscreenWindow {
                    handle,
                    restore: restore.clone(),
                });
                cx.notify();
                true
            });
            if !installed {
                restore.set(false);
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
        }
        Err(_) => owner.update(cx, |player, cx| {
            if player.fullscreen_generation == generation {
                player.fullscreen_opening = false;
                player.local_error = Some(t("media.fullscreenFailed"));
                cx.notify();
            }
        }),
    }
}

impl MediaPlayer {
    pub(super) fn open_fullscreen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled
            || self.spec.kind != MediaKind::Video
            || self.current.frame.is_none()
            || self.fullscreen_opening
        {
            return;
        }
        if let Some(owner) = &self.fullscreen {
            let handle = owner.handle;
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, _| window.activate_window());
            });
            return;
        }
        self.fullscreen_generation = self.fullscreen_generation.wrapping_add(1);
        self.fullscreen_opening = true;
        let generation = self.fullscreen_generation;
        let player = cx.entity().downgrade();
        let opener = window.window_handle();
        let previous = window.focused(cx).map(|focus| focus.downgrade());
        let display = window.display(cx);
        let bounds = display
            .as_ref()
            .map(|display| display.bounds())
            .unwrap_or_else(|| window.bounds());
        let display_id = display.map(|display| display.id());
        // Creating a window can synchronously draw its view. Release the
        // MediaPlayer update borrow before that view reads the same player.
        cx.defer(move |cx| {
            create_window(player, generation, opener, previous, bounds, display_id, cx)
        });
    }
    pub(super) fn close_fullscreen(&mut self, cx: &mut Context<Self>) {
        release_window(self, cx);
        cx.notify();
    }
    pub(super) fn exit_fullscreen(&mut self, cx: &mut Context<Self>) {
        if let Some(owner) = self.fullscreen.take() {
            cx.defer(move |cx| {
                let _ = owner
                    .handle
                    .update(cx, |_, window, _| window.remove_window());
            });
            cx.notify();
        }
    }
    pub(super) fn fullscreen_controls(
        &self,
        theme: Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.controls(
            theme,
            true,
            true,
            f32::from(window.viewport_size().width),
            window,
            cx,
        )
    }
}
impl Render for VideoFullscreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(player) = self.player.upgrade() else {
            window.remove_window();
            return div().into_any_element();
        };
        let current = Theme::current(cx);
        let mut theme = Theme::dark();
        theme.accent = current.accent;
        theme.accent_text = current.accent_text;
        theme.focus = current.focus;
        theme.focus_ring = current.focus_ring;
        let frame = player.read(cx).current.frame.clone();
        let weak = player.downgrade();
        let controls = player.update(cx, |player, cx| {
            player.sync_sliders(window, cx);
            player.fullscreen_controls(theme, window, cx)
        });
        div()
            .id("video-fullscreen")
            .relative()
            .size_full()
            .bg(theme.media_surface)
            .text_color(theme.text)
            .font_family(stonks_ui::fonts::FAMILY)
            .role(Role::Dialog)
            .aria_label(t("media.fullscreenVideo"))
            .occlude()
            .capture_key_down(cx.listener(|_, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    window.remove_window();
                    window.prevent_default();
                    cx.stop_propagation();
                }
            }))
            .when_some(frame, |stage, frame| {
                stage.child(
                    img(frame)
                        .id("fullscreen-video-frame")
                        .size_full()
                        .object_fit(ObjectFit::Contain),
                )
            })
            .on_click(move |event, _, cx| {
                if event.click_count() == 2 {
                    weak.update(cx, |player, cx| player.exit_fullscreen(cx))
                        .ok();
                }
            })
            .child(
                div()
                    .id("fullscreen-controls")
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .px(px(theme.space4))
                    .py(px(theme.space2))
                    .bg(theme.media_chrome)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(controls),
            )
            .focus_trap("fullscreen-video-focus", &self.focus)
            .into_any_element()
    }
}
