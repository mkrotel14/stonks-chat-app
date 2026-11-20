//! Geometry and resize interactions translated from sidebarResize.ts.
use super::*;
use crate::desktop::explorer_layout::{self, CHANNEL_MAX, CHANNEL_MIN};
pub(super) fn fitted_widths(
    viewport: f32,
    rail: f32,
    channel: f32,
    member: f32,
    member_open: bool,
) -> (f32, f32) {
    let available = (viewport - rail - 360.).max(0.);
    let requested = if member_open { member } else { 0. };
    if channel + requested <= available {
        return (channel, member);
    }
    if member_open && member >= 150. {
        let room = available - channel;
        if room >= 150. {
            return (channel, member.min(room.floor()));
        }
        let room = available - 150.;
        if room >= 150. {
            return (channel.min(room.floor()), 150.);
        }
    }
    let channel = if channel >= 150. && available >= 150. {
        channel.min(available.floor())
    } else {
        0.
    };
    (channel, if member_open { 0. } else { member })
}
fn width(raw: f32, available: f32, maximum: f32) -> f32 {
    if raw < 150. || available < 150. {
        0.
    } else {
        raw.min(available).min(maximum).round()
    }
}
// Unlike an intentional collapse, lack of room should not update the preferred width.
fn channel_resize_width(raw: f32, available: f32) -> Option<f32> {
    (available >= CHANNEL_MIN).then(|| width(raw, available, CHANNEL_MAX))
}

use std::time::{Duration, Instant};

const EDGE_WIDTH: f32 = 8.;
const RAIL_TRANSITION: Duration = Duration::from_millis(150);

#[derive(Default)]
pub(super) struct ServerRail {
    hidden: bool,
    hovered: bool,
    transition: Option<(Instant, f32)>,
    drag_start: Option<f32>,
}

impl ServerRail {
    pub(super) fn new(hidden: bool) -> Self {
        Self {
            hidden,
            ..Self::default()
        }
    }

    fn visible_width(&self, now: Instant) -> f32 {
        let target = if self.hovered { RAIL_WIDTH } else { 0. };
        let Some((start, from)) = self.transition else {
            return target;
        };
        let t =
            (now.duration_since(start).as_secs_f32() / RAIL_TRANSITION.as_secs_f32()).clamp(0., 1.);
        // Fast ease-out, approximating the Tauri standard easing.
        from + (target - from) * (1. - (1. - t).powi(3))
    }
}

impl ShellView {
    pub(super) fn fitted_explorer_widths(&self, viewport: f32) -> (f32, f32) {
        fitted_widths(
            viewport,
            self.rail_width(),
            if self.show_channels {
                self.channel_width
            } else {
                0.
            },
            self.member_width,
            self.show_members && !self.home,
        )
    }

    pub(super) fn set_channels_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.show_channels = open;
        explorer_layout::update(cx, |layout| layout.channel_collapsed = !open);
    }

    fn resize_channels(&mut self, next: f32, cx: &mut Context<Self>) {
        self.set_channels_open(next > 0., cx);
        if next > 0. {
            self.channel_width = next;
            explorer_layout::update(cx, |layout| layout.channel_expanded_width = Some(next));
        }
    }

    pub(super) fn rail_hidden(&self) -> bool {
        self.server_rail.hidden
    }

    pub(super) fn rail_width(&self) -> f32 {
        if self.rail_hidden() {
            0.
        } else {
            RAIL_WIDTH
        }
    }

    pub(super) fn set_rail_hidden(&mut self, hidden: bool, cx: &mut Context<Self>) {
        if self.rail_hidden() != hidden {
            self.server_rail.hidden = hidden;
            explorer_layout::update(cx, |layout| layout.rail_hidden = hidden);
            self.server_rail.hovered = false;
            self.server_rail.transition = None;
            cx.notify();
        }
    }

    pub(super) fn reset_rail_preview(&mut self) {
        self.server_rail.hovered = false;
        self.server_rail.transition = None;
        self.server_rail.drag_start = None;
    }

    pub(super) fn rail_preview(&mut self, window: &Window) -> f32 {
        if !self.rail_hidden() || !window.is_window_active() {
            self.server_rail.hovered = false;
            self.server_rail.transition = None;
            return 0.;
        }
        let now = Instant::now();
        if let Some((start, _)) = self.server_rail.transition {
            if now.duration_since(start) < RAIL_TRANSITION {
                window.request_animation_frame();
            } else {
                self.server_rail.transition = None;
            }
        }
        self.server_rail.visible_width(now)
    }

    pub(super) fn rail_edge(
        &self,
        rail: Option<AnyElement>,
        visible: f32,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id("server-rail-edge")
            .absolute()
            .left_0()
            .top_0()
            .h_full()
            .w(px(visible + EDGE_WIDTH))
            .overflow_hidden()
            .occlude()
            .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                if !this.rail_hidden()
                    || this.focused
                    || !window.is_window_active()
                    || this.server_rail.hovered == *hovered
                {
                    return;
                }
                if cx.reduce_motion() {
                    this.server_rail.hovered = *hovered;
                    this.server_rail.transition = None;
                    cx.notify();
                    return;
                }
                let now = Instant::now();
                let from = this.server_rail.visible_width(now);
                this.server_rail.hovered = *hovered;
                this.server_rail.transition = Some((now, from));
                cx.notify();
            }))
            .when_some(rail, |edge, rail| {
                edge.child(
                    div()
                        .absolute()
                        .left(px(visible - RAIL_WIDTH))
                        .top_0()
                        .w(px(RAIL_WIDTH))
                        .h_full()
                        .child(rail),
                )
                .child(self.rail_grip(visible, theme, cx))
            })
    }

    fn rail_grip(&self, right_edge: f32, theme: Theme, cx: &mut Context<Self>) -> Div {
        div()
            .absolute()
            .top_0()
            .left(px(right_edge - 4.5))
            .h_full()
            .w(px(9.))
            .child(
                stonks_ui::components::UiResizeGrip::new("server-rail-grip", theme)
                    .active(self.server_rail.drag_start.is_some())
                    .on_mouse_down(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                        this.server_rail.drag_start = Some(f32::from(event.position.x));
                        cx.stop_propagation();
                        cx.notify();
                    }))
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        let hidden = match event.keystroke.key.as_str() {
                            "home" | "left" => true,
                            "end" | "right" => false,
                            _ => return,
                        };
                        this.set_rail_hidden(hidden, cx);
                        window.prevent_default();
                        cx.stop_propagation();
                    })),
            )
    }

    pub(super) fn resize_handles(
        &mut self,
        root: Stateful<Div>,
        channel: f32,
        member: f32,
        channels_visible: bool,
        members_visible: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let mut root = root
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                let Some(drag) = this.drag.as_ref() else {
                    return;
                };
                if !window.is_window_active() || event.pressed_button != Some(MouseButton::Left) {
                    this.drag = None;
                    cx.notify();
                    return;
                }
                let member = drag.member;
                let delta =
                    (f32::from(event.position.x) - drag.start_x) * if member { -1. } else { 1. };
                let raw = drag.start_width + delta;
                let other = if member {
                    if this.show_channels {
                        this.channel_width
                    } else {
                        0.
                    }
                } else if this.show_members && !this.home {
                    this.fitted_explorer_widths(f32::from(window.viewport_size().width))
                        .1
                } else {
                    0.
                };
                let available =
                    (f32::from(window.viewport_size().width) - this.rail_width() - 360. - other)
                        .max(0.);
                if member {
                    let next = width(raw, available, 360.);
                    this.show_members = next > 0.;
                    if next > 0. {
                        this.member_width = next;
                    }
                } else if let Some(next) = channel_resize_width(raw, available) {
                    if next
                        != this
                            .fitted_explorer_widths(f32::from(window.viewport_size().width))
                            .0
                    {
                        this.resize_channels(next, cx);
                    }
                }
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.drag = None;
                    cx.notify();
                }),
            );
        if let Some(start) = self.server_rail.drag_start {
            // An occluding preview can cover the shell hitbox while the grip is dragged.
            let move_rail = cx.listener(move |this, event: &MouseMoveEvent, window, cx| {
                if this.server_rail.drag_start.is_none() {
                    return;
                }
                if !window.is_window_active() || event.pressed_button != Some(MouseButton::Left) {
                    this.server_rail.drag_start = None;
                    cx.notify();
                } else {
                    let delta = f32::from(event.position.x) - start;
                    if delta <= -20. || delta >= 20. {
                        this.set_rail_hidden(delta < 0., cx);
                    }
                }
            });
            let end_drag = cx.listener(|this, _: &MouseUpEvent, _, cx| {
                this.server_rail.drag_start = None;
                cx.notify();
            });
            root = root.child(
                canvas(
                    |_, _, _| {},
                    move |_, _, window, _| {
                        window.on_mouse_event(move |event, phase, window, cx| {
                            if phase == DispatchPhase::Capture {
                                move_rail(event, window, cx);
                            }
                        });
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                            if phase == DispatchPhase::Capture && event.button == MouseButton::Left
                            {
                                end_drag(event, window, cx);
                            }
                        });
                    },
                )
                .absolute()
                .size_0(),
            );
        }
        if !self.rail_hidden() {
            root = root.child(self.rail_grip(RAIL_WIDTH, *theme, cx));
        }
        for (is_member, visible, current) in [
            (false, channels_visible, channel),
            (true, members_visible, member),
        ] {
            if !visible {
                continue;
            }
            let handle = stonks_ui::components::UiResizeGrip::new(
                if is_member {
                    "member-grip"
                } else {
                    "channel-grip"
                },
                *theme,
            )
            .active(
                self.drag
                    .as_ref()
                    .is_some_and(|drag| drag.member == is_member),
            )
            .on_mouse_down(cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                this.drag = Some(SidebarDrag {
                    member: is_member,
                    start_x: f32::from(event.position.x),
                    start_width: current,
                });
                cx.stop_propagation();
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                let direction = if is_member { -1. } else { 1. };
                let maximum = if is_member { 360. } else { CHANNEL_MAX };
                let viewport = f32::from(window.viewport_size().width);
                let other = if !is_member && this.show_members && !this.home {
                    this.fitted_explorer_widths(viewport).1
                } else {
                    0.
                };
                let available = viewport - this.rail_width() - 360. - other;
                let next = match event.keystroke.key.as_str() {
                    "home" => 0.,
                    "end" => width(maximum, available, maximum),
                    "left" => width(current - 16. * direction, available, maximum),
                    "right" => width(current + 16. * direction, available, maximum),
                    _ => return,
                };
                if is_member {
                    this.show_members = next > 0.;
                    if next > 0. {
                        this.member_width = next;
                    }
                } else if available >= CHANNEL_MIN || event.keystroke.key == "home" {
                    this.resize_channels(next, cx);
                }
                window.prevent_default();
                cx.stop_propagation();
                cx.notify();
            }));
            root = root.child(
                div()
                    .absolute()
                    .top_0()
                    .h_full()
                    .w(px(9.))
                    .when(is_member, |handle| handle.right(px(member - 4.5)))
                    .when(!is_member, |handle| {
                        handle.left(px(self.rail_width() + channel - 4.5))
                    })
                    .child(handle),
            );
        }
        root
    }
}
#[cfg(test)]
mod tests {
    use super::RAIL_WIDTH;
    use super::{channel_resize_width, fitted_widths, width};

    #[test]
    fn responsive_fitting_is_not_a_channel_collapse() {
        assert_eq!(fitted_widths(540., RAIL_WIDTH, 320., 280., false).0, 0.);
        assert_eq!(channel_resize_width(320., 124.), None);
        assert_eq!(channel_resize_width(149., 400.), Some(0.));
        assert_eq!(fitted_widths(1200., RAIL_WIDTH, 320., 280., false).0, 320.);
    }

    #[test]
    fn preserves_conversation_and_source_fitting_order() {
        assert_eq!(
            fitted_widths(1200., RAIL_WIDTH, 292., 280., true),
            (292., 280.)
        );
        assert_eq!(fitted_widths(938., 0., 292., 280., true), (292., 280.));
        assert_eq!(
            fitted_widths(938., RAIL_WIDTH, 292., 280., true),
            (292., 230.)
        );
        assert_eq!(
            fitted_widths(800., RAIL_WIDTH, 292., 280., true),
            (234., 150.)
        );
        assert_eq!(
            fitted_widths(650., RAIL_WIDTH, 292., 280., true),
            (234., 0.)
        );
        assert_eq!(width(149., 400., 380.), 0.);
        assert_eq!(width(500., 300., 380.), 300.);
    }
}
