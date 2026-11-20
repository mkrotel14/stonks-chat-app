//! Source centered message navigation and 2.8-second target highlight.
use gpui_kit::*;
use std::time::Duration;
use stonks_ui::Theme;
pub(crate) const HIGHLIGHT_DURATION: Duration = Duration::from_millis(2800);
pub(crate) const SCROLL_FRAMES: u32 = 14;
#[derive(Default)]
pub(crate) struct State {
    pub revision: u64,
    pub center_pending: Option<String>,
    pub highlighted: Option<String>,
    pub scroll: Option<Task<()>>,
    pub clear: Option<Task<()>>,
}
impl State {
    pub fn reset(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.center_pending = None;
        self.highlighted = None;
        self.scroll = None;
        self.clear = None;
    }
    pub fn begin(&mut self, id: String) -> u64 {
        self.reset();
        self.center_pending = Some(id.clone());
        self.highlighted = Some(id);
        self.revision
    }
    pub fn stop_scroll(&mut self) {
        self.center_pending = None;
        self.scroll = None;
    }
}
pub(crate) fn center_distance(row: Bounds<Pixels>, viewport: Bounds<Pixels>) -> f32 {
    f32::from(row.center().y - viewport.center().y)
}
pub(crate) fn scroll_center_step(list: &ListState, distance: f32) {
    // Resolve the pixel displacement to the actual top row. A negative
    // offset on the target row leaves a gap: GPUI only paints from item_ix.
    list.scroll_by(px(distance));
}
pub(crate) fn smooth_offset(start: f32, end: f32, frame: u32) -> f32 {
    let progress = (frame as f32 / SCROLL_FRAMES as f32).clamp(0., 1.);
    start + (end - start) * (1. - (1. - progress).powi(3))
}
pub(crate) fn highlight(theme: Theme, id: &str, revision: u64, reduced_motion: bool) -> AnyElement {
    let overlay = div()
        .absolute()
        .inset_0()
        .rounded(px(theme.radius_sm))
        .bg(theme.surface_selected)
        .border_l(px(3.))
        .border_color(theme.accent);
    if reduced_motion {
        overlay.into_any_element()
    } else {
        overlay
            .with_animation(
                ElementId::Name(format!("{id}-target-highlight-{revision}").into()),
                Animation::new(HIGHLIGHT_DURATION)
                    .with_easing(|progress| ((progress - 0.72) / 0.28).clamp(0., 1.)),
                |overlay, phase| overlay.opacity(1. - phase),
            )
            .into_any_element()
    }
}
#[cfg(test)]
mod tests {
    use super::{
        center_distance, scroll_center_step, smooth_offset, State, HIGHLIGHT_DURATION,
        SCROLL_FRAMES,
    };
    use gpui_kit::{point, px, size, Bounds, ListAlignment, ListOffset, ListState};
    #[test]
    fn center_uses_actual_painted_message_bounds_and_smoothly_reaches_target() {
        let viewport = Bounds {
            origin: point(px(0.), px(100.)),
            size: size(px(500.), px(600.)),
        };
        let row = Bounds {
            origin: point(px(0.), px(110.)),
            size: size(px(500.), px(40.)),
        };
        let distance = center_distance(row, viewport);
        assert_eq!(distance, -270.);
        assert_eq!(row.center().y - px(distance), viewport.center().y);
        assert_eq!(smooth_offset(0., distance, 0), 0.);
        assert!(smooth_offset(0., distance, 7) < 0.);
        assert!(smooth_offset(0., distance, 7) > distance);
        assert_eq!(smooth_offset(0., distance, SCROLL_FRAMES), distance);
        assert_eq!(HIGHLIGHT_DURATION.as_millis(), 2800);
    }
    #[test]
    fn centering_past_loaded_start_clamps_instead_of_leaving_a_negative_row_offset() {
        let list = ListState::new(3, ListAlignment::Bottom, px(300.));
        list.scroll_to(ListOffset {
            item_ix: 1,
            offset_in_item: px(0.),
        });
        scroll_center_step(&list, -270.);
        assert!(list.logical_scroll_top().offset_in_item >= px(0.));
    }
    #[test]
    fn newer_jumps_and_channel_reset_invalidate_old_center_and_highlight_work() {
        let mut state = State::default();
        let first = state.begin("first".into());
        let second = state.begin("second".into());
        assert_ne!(first, second);
        assert_eq!(state.center_pending.as_deref(), Some("second"));
        state.stop_scroll();
        assert_eq!(state.center_pending, None);
        assert_eq!(state.highlighted.as_deref(), Some("second"));
        state.reset();
        assert_ne!(state.revision, second);
        assert_eq!(state.highlighted, None);
    }
}
