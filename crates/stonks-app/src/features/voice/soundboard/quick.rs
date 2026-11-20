//! Dock-anchored quick playback.
use super::*;

impl Voice {
    pub(super) fn quick_soundboard(
        &self,
        panel: &Panel,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let anchor = panel.anchor?;
        let (width, left, edge, height, above) = popup_geometry(anchor, window.viewport_size());
        let theme = Theme::current(cx);
        let mut popup = div()
            .id("quick-sounds")
            .role(Role::Dialog)
            .aria_label(t("settings.pages.soundboard"))
            .absolute()
            .left(left)
            .w(width)
            .when(above, |el| el.bottom(edge))
            .when(!above, |el| el.top(edge))
            .max_h(height)
            .flex()
            .flex_col()
            .gap(px(theme.space3))
            .p(px(theme.space3))
            .border_1()
            .border_color(theme.outline)
            .rounded(px(theme.radius_lg))
            .bg(theme.surface_raised)
            .shadow(vec![BoxShadow {
                color: theme.shadow_md.color.into(),
                offset: point(px(0.), px(theme.shadow_md.dy)),
                blur_radius: px(theme.shadow_md.blur),
                spread_radius: px(0.),
                inset: false,
            }])
            .occlude()
            .on_mouse_down_out(
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    if !anchor.contains(&event.position) && this.quick_sounds_open() {
                        this.close_soundboard(window, cx);
                    }
                }),
            )
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme.space2))
                    .flex_shrink_0()
                    .child(div().flex_1().min_w_0().child(
                        SettingsInput::new(&panel.name, theme).label(t("voice.searchSounds")),
                    ))
                    .child(Icon::Search.render(theme.text_muted, 20.)),
            )
            .child(self.soundboard_volume_control(panel, theme));
        if let Some(library) = &panel.library {
            let query = panel.name.read(cx).value().trim().to_lowercase();
            let clips: Vec<_> = library
                .clips
                .iter()
                .filter(|clip| clip.name.to_lowercase().contains(&query))
                .collect();
            if clips.is_empty() {
                popup = popup.child(t(if library.clips.is_empty() {
                    "voice.addSoundsHint"
                } else {
                    "voice.noSoundMatches"
                }));
            } else {
                let columns = if width >= px(280.) {
                    4
                } else if width >= px(185.) {
                    3
                } else {
                    2
                };
                let disabled = !self.can_play_sound();
                popup = popup.child(
                    div()
                        .id("quick-sounds-grid")
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .min_h_0()
                        .max_h(px(184.))
                        .overflow_y_scroll()
                        .children(clips.chunks(columns).map(|row| {
                            div()
                                .flex()
                                .flex_shrink_0()
                                .gap(px(6.))
                                .children((0..columns).map(|index| {
                                    let mut cell = div().flex_1().min_w_0();
                                    if let Some(clip) = row.get(index) {
                                        let id = clip.id.clone();
                                        let label =
                                            t_vars("voice.playSound", &[("name", &clip.name)]);
                                        cell = cell.child(
                                            Button::new(
                                                SharedString::from(format!("quick-sound-{id}")),
                                                theme,
                                            )
                                            .variant(ButtonVariant::Secondary)
                                            .tooltip(label.clone())
                                            .accessibility_label(label)
                                            .disabled(disabled)
                                            .leading(
                                                div().min_w_0().truncate().child(clip.name.clone()),
                                            )
                                            .on_click(
                                                cx.listener(move |this, _, _, cx| {
                                                    this.play_sound(id.clone(), false, cx)
                                                }),
                                            ),
                                        );
                                    }
                                    cell
                                }))
                        })),
                );
            }
        } else if panel.work.is_some() {
            popup = popup.child(t("common.loading"));
        } else {
            popup = popup.child(
                Button::new("quick-sounds-retry", theme)
                    .variant(ButtonVariant::Secondary)
                    .label(t("common.retry"))
                    .on_click(
                        cx.listener(|this, _, window, cx| this.change(Change::Refresh, window, cx)),
                    ),
            );
        }
        if let Some(error) = &panel.error {
            popup = popup.child(Feedback::new(theme, FeedbackTone::Error, error.clone()));
        }
        Some(popup.into_any_element())
    }
}

fn popup_geometry(
    anchor: Bounds<Pixels>,
    viewport: Size<Pixels>,
) -> (Pixels, Pixels, Pixels, Pixels, bool) {
    let margin = px(12.).min(viewport.width / 4.);
    let width = px(380.).min(viewport.width - margin * 2.);
    let left = (anchor.right() - width).clamp(margin, viewport.width - width - margin);
    let above_room = (anchor.top() - px(20.)).max(px(0.));
    let below_room = (viewport.height - anchor.bottom() - px(20.)).max(px(0.));
    let above = above_room >= px(310.).min(below_room);
    if above {
        (
            width,
            left,
            viewport.height - anchor.top() + px(8.),
            above_room,
            true,
        )
    } else {
        (width, left, anchor.bottom() + px(8.), below_room, false)
    }
}

#[cfg(test)]
mod tests {
    use super::popup_geometry;
    use gpui_kit::{point, px, size, Bounds};

    #[test]
    fn popup_aligns_right_above_dock_and_clamps_to_window() {
        let viewport = size(px(1000.), px(800.));
        let anchor = Bounds::new(point(px(830.), px(620.)), size(px(80.), px(40.)));
        assert_eq!(
            popup_geometry(anchor, viewport),
            (px(380.), px(530.), px(188.), px(600.), true)
        );
        let anchor = Bounds::new(point(px(20.), px(30.)), size(px(80.), px(40.)));
        assert_eq!(
            popup_geometry(anchor, size(px(300.), px(600.))),
            (px(276.), px(12.), px(78.), px(510.), false)
        );
    }
}
