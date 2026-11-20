//! Paint-only motion for shell surfaces and disclosure controls.

use super::*;
use std::time::Duration;

pub(super) fn entrance<E: IntoElement + Styled + 'static>(
    element: E,
    id: impl Into<ElementId>,
) -> impl IntoElement {
    // GPUI resolves one-shot animations to their final frame under reduce_motion.
    element.with_animation(
        id,
        Animation::new(Duration::from_millis(150)).with_easing(|phase| 1. - (1. - phase).powi(3)),
        |element, phase| element.opacity(0.82 + 0.18 * phase),
    )
}

pub(super) fn account_settings(
    layer: Div,
    settings: &Entity<crate::features::settings::Settings>,
) -> impl IntoElement {
    entrance(
        layer.child(settings.clone()),
        ("account-settings-entrance", settings.entity_id()),
    )
}

pub(super) fn disclosure(
    id: impl Into<ElementId>,
    collapsed: bool,
    color: Rgba,
    size: f32,
    animate: bool,
) -> AnyElement {
    let icon = |collapsed| {
        if collapsed {
            Icon::ChevronRight
        } else {
            Icon::ChevronDown
        }
    };
    let slot = div().relative().size(px(size)).flex_shrink_0();
    if !animate {
        return slot
            .child(icon(collapsed).render(color, size))
            .into_any_element();
    }
    // Only the icons crossfade; the button and its rows change state immediately.
    slot.with_spring(
        id,
        SpringAnimation::new(SpringConfig::new(1600., 80., 1.))
            .to(if collapsed { 0. } else { 1. })
            .with_epsilon(0.01),
        move |slot, phase: f32| {
            let phase = phase.clamp(0., 1.);
            slot.child(
                div()
                    .absolute()
                    .inset_0()
                    .opacity(1. - phase)
                    .child(icon(true).render(color, size)),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .opacity(phase)
                    .child(icon(false).render(color, size)),
            )
        },
    )
    .into_any_element()
}
