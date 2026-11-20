use super::*;

pub(in crate::features::voice) struct Overflow {
    position: Point<Pixels>,
    focus: FocusHandle,
    opener: FocusHandle,
}

fn visible_limit(width: Pixels, count: usize) -> usize {
    let preferred = if width >= px(210.) { 6 } else { 2 };
    let slots = ((f32::from(width) + 5.) / 33.).floor().max(1.) as usize;
    if count <= slots {
        preferred
    } else {
        preferred.min(slots - 1)
    }
}

fn measure_opener(button: Button, bounds: Rc<Cell<Option<Bounds<Pixels>>>>) -> Div {
    div().relative().flex_shrink_0().child(button).child(
        canvas(move |area, _, _| bounds.set(Some(area)), |_, _, _, _| {})
            .absolute()
            .inset_0(),
    )
}

impl Voice {
    pub(in crate::features::voice) fn clear_dock_roster(&mut self) {
        self.dock_overflow = None;
        if self
            .participant_volume
            .as_ref()
            .is_some_and(|panel| panel.in_dock)
        {
            self.participant_volume = None;
        }
    }

    pub(in crate::features::voice) fn dock_participants(
        &mut self,
        channel: &str,
        entries: &[Participant],
        width: Pixels,
        images: &Entity<Images>,
        theme: Theme,
        viewport: Size<Pixels>,
        cx: &mut Context<Self>,
    ) -> Div {
        let limit = visible_limit(width, entries.len()).min(entries.len());
        let (visible, hidden) = entries.split_at(limit);
        if hidden.is_empty() {
            self.dock_overflow = None;
        }
        if self.participant_volume.as_ref().is_some_and(|panel| {
            panel.in_dock
                && (panel.channel != channel || !entries.iter().any(|entry| entry.id == panel.id))
        }) {
            self.participant_volume = None;
        }
        let panel = self
            .participant_volume
            .as_ref()
            .filter(|panel| panel.in_dock);
        let voice = cx.entity();
        let avatar_for = |entry: &Participant, edge, cx: &mut App| {
            let image = images.update(cx, |images, cx| {
                images.get(entry.avatar_url.as_deref(), AVATAR_EDGE, cx)
            });
            div()
                .relative()
                .flex_shrink_0()
                .child(avatar(&theme, &entry.name, edge, image))
                .when(entry.speaking, |ring| {
                    ring.child(
                        div()
                            .absolute()
                            .top(px(-2.))
                            .left(px(-2.))
                            .size(px(edge + 4.))
                            .rounded_full()
                            .border_2()
                            .border_color(theme.success),
                    )
                })
        };
        let mut strip = div().relative().flex().items_center().gap(px(5.));
        let overflow_open = self.dock_overflow.is_some();
        let overflow_opener = self
            .dock_overflow
            .as_ref()
            .map(|open| open.opener.clone())
            .or_else(|| {
                panel
                    .filter(|panel| hidden.iter().any(|entry| entry.id == panel.id))
                    .map(|panel| panel.opener.clone())
            })
            .unwrap_or_else(|| cx.focus_handle());
        let mut volume_opener_bounds = None;
        let overflow_bounds = Rc::new(Cell::new(None::<Bounds<Pixels>>));
        for entry in visible {
            let image = avatar_for(entry, 24., cx);
            let id = SharedString::from(format!("dock-avatar-{}", entry.id));
            if entry.is_self {
                let name = SharedString::from(entry.name.clone());
                strip = strip.child(
                    div()
                        .id(id)
                        .aria_label(name.clone())
                        .tooltip(move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(name.clone())
                                .build(window, cx)
                        })
                        .child(image),
                );
                continue;
            }
            let open = panel.filter(|panel| panel.id == entry.id);
            let focus = open
                .map(|panel| panel.opener.clone())
                .unwrap_or_else(|| cx.focus_handle());
            let (channel, participant) = (channel.to_owned(), entry.id.clone());
            let bounds = Rc::new(Cell::new(None::<Bounds<Pixels>>));
            if open.is_some() {
                volume_opener_bounds = Some(bounds.clone());
            }
            let anchor = bounds.clone();
            strip = strip.child(measure_opener(
                Button::new(id, theme)
                    .variant(ButtonVariant::Quiet)
                    .icon_button_size(28.)
                    .leading(image)
                    .tooltip(entry.name.clone())
                    .expanded(open.is_some())
                    .track_focus(&focus)
                    .accessibility_label(t_vars("voice.volumeFor", &[("name", &entry.name)]))
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        if this
                            .participant_volume
                            .as_ref()
                            .is_some_and(|panel| panel.in_dock && panel.id == participant)
                        {
                            this.close_participant_volume(true, window, cx);
                        } else {
                            this.open_participant_menu(
                                &channel,
                                &participant,
                                anchor
                                    .get()
                                    .map_or_else(|| event.position(), |area| area.origin),
                                &focus,
                                true,
                                window,
                                cx,
                            );
                        }
                    })),
                bounds,
            ));
        }
        if !hidden.is_empty() {
            let opener = overflow_opener.clone();
            let anchor = overflow_bounds.clone();
            strip = strip.child(measure_opener(
                Button::new("dock-more-participants", theme)
                    .variant(ButtonVariant::Secondary)
                    .icon_button_size(28.)
                    .label(format!("+{}", hidden.len()))
                    .expanded(overflow_open)
                    .track_focus(&opener)
                    .accessibility_label(t_vars(
                        "voice.showMoreParticipants",
                        &[("count", &hidden.len().to_string())],
                    ))
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        if this.dock_overflow.take().is_some() {
                            opener.focus(window, cx);
                        } else {
                            this.participant_volume = None;
                            let focus = cx.focus_handle();
                            focus.focus(window, cx);
                            this.dock_overflow = Some(Overflow {
                                position: anchor
                                    .get()
                                    .map_or_else(|| event.position(), |area| area.origin),
                                focus,
                                opener: opener.clone(),
                            });
                        }
                        cx.notify();
                    })),
                overflow_bounds.clone(),
            ));
        }
        if let Some(open) = &self.dock_overflow {
            let (position, focus) = (open.position, open.focus.clone());
            let outside = voice.clone();
            let bounds = overflow_bounds;
            let mut list = div()
                .id("dock-participant-overflow")
                .role(Role::Dialog)
                .aria_label(t("voice.moreParticipants"))
                .track_focus(&focus)
                .occlude()
                .w(px(240.).min((viewport.width - px(24.)).max(px(1.))))
                .max_h(px(360.).min(viewport.height * 0.6))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(px(theme.space1))
                .p(px(theme.space2))
                .rounded(px(theme.radius_md))
                .border_1()
                .border_color(theme.outline)
                .bg(theme.surface_raised)
                .on_mouse_down_out(move |event, _, cx| {
                    if bounds
                        .get()
                        .is_some_and(|area| area.contains(&event.position))
                    {
                        return;
                    }
                    outside.update(cx, |this, cx| {
                        this.dock_overflow = None;
                        cx.notify();
                    })
                })
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" {
                        if let Some(open) = this.dock_overflow.take() {
                            open.opener.focus(window, cx);
                            cx.notify();
                        }
                        window.prevent_default();
                        cx.stop_propagation();
                    }
                }));
            for entry in hidden {
                let row = div()
                    .flex()
                    .items_center()
                    .gap(px(theme.space2))
                    .w_full()
                    .min_w_0()
                    .child(div().flex_shrink_0().child(avatar_for(entry, 28., cx)))
                    .child(div().min_w_0().truncate().child(entry.name.clone()));
                if entry.is_self {
                    list = list.child(row.px(px(theme.space2)));
                } else {
                    let (channel, participant, opener) = (
                        channel.to_owned(),
                        entry.id.clone(),
                        overflow_opener.clone(),
                    );
                    list = list.child(
                        Button::new(SharedString::from(entry.id.clone()), theme)
                            .variant(ButtonVariant::MemberAvatar)
                            .leading(row)
                            .accessibility_label(t_vars(
                                "voice.volumeFor",
                                &[("name", &entry.name)],
                            ))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.dock_overflow = None;
                                this.open_participant_menu(
                                    &channel,
                                    &participant,
                                    position,
                                    &opener,
                                    true,
                                    window,
                                    cx,
                                );
                            })),
                    );
                }
            }
            strip = strip.child(
                div().absolute().inset_0().child(
                    deferred(
                        anchored()
                            .anchor(Anchor::BottomLeft)
                            .position_mode(AnchoredPositionMode::Window)
                            .position(position)
                            .snap_to_window_with_margin(px(12.))
                            .child(list),
                    )
                    .with_priority(1),
                ),
            );
        }
        if let Some(entry) =
            panel.and_then(|panel| entries.iter().find(|entry| entry.id == panel.id))
        {
            strip = strip.child(ParticipantMenu {
                voice,
                name: entry.name.clone(),
                theme,
                opener_bounds: volume_opener_bounds,
            });
        }
        strip
    }
}

#[cfg(test)]
mod tests {
    use super::{px, visible_limit};

    #[test]
    fn strip_uses_tauri_breakpoint_and_reserves_overflow_space() {
        assert_eq!(visible_limit(px(209.), 10), 2);
        assert_eq!(visible_limit(px(210.), 10), 5);
        assert_eq!(visible_limit(px(210.), 6), 6);
        assert_eq!(visible_limit(px(226.), 10), 6);
        assert_eq!(visible_limit(px(94.), 5), 2);
        assert_eq!(visible_limit(px(65.), 5), 1);
        assert_eq!(visible_limit(px(28.), 5), 0);
    }
}
