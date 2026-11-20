//! Reading states and geometry from MessageTimeline.tsx/MessageComposer.stylex.ts.
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use stonks_ui::{
    i18n::{t, t_vars},
    Theme,
};

pub(crate) const TOP_PADDING: f32 = 16.;
pub(crate) const COMPOSER_INSET: f32 = 92.;
pub(crate) const PINNED_THRESHOLD: f32 = 80.;
pub(crate) const NEAR_BOTTOM_THRESHOLD: f32 = 120.;
pub(crate) const NEAR_START_THRESHOLD: f32 = 140.;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ChannelIntro {
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum EmptyState {
    NoChannel,
    Loading,
    Failed,
    NoHistory,
    Empty,
}
pub(crate) fn empty_state(
    selected: bool,
    loading: bool,
    failed: bool,
    can_read: bool,
) -> EmptyState {
    match (selected, loading, failed, can_read) {
        (false, _, _, _) => EmptyState::NoChannel,
        (_, true, _, _) => EmptyState::Loading,
        (_, _, true, _) => EmptyState::Failed,
        (_, _, _, false) => EmptyState::NoHistory,
        _ => EmptyState::Empty,
    }
}
pub(crate) fn show_intro(loading: bool, has_more: bool, count: usize, name: Option<&str>) -> bool {
    !loading && !has_more && count > 0 && name.is_some_and(|name| !name.is_empty())
}
pub(crate) fn first_day_divider(has_more: bool, name: Option<&str>) -> bool {
    has_more || name.is_none_or(str::is_empty)
}
pub(crate) fn grouped(
    same_author: bool,
    previous_system: bool,
    current_system: bool,
    gap_ms: Option<u64>,
) -> bool {
    same_author
        && !previous_system
        && !current_system
        && gap_ms.is_some_and(|gap| gap < 7 * 60 * 1000)
}
#[derive(Debug, PartialEq)]
pub(crate) struct RowMetrics {
    pub top: f32,
    pub bottom: f32,
    pub margin: f32,
}
pub(crate) fn row_metrics(compact: bool, grouped: bool) -> RowMetrics {
    if compact {
        RowMetrics {
            top: 1.,
            bottom: 2.,
            margin: 0.,
        }
    } else if grouped {
        RowMetrics {
            top: 2.,
            bottom: 2.,
            margin: 3.,
        }
    } else {
        RowMetrics {
            top: 5.,
            bottom: 6.,
            margin: 3.,
        }
    }
}
pub(crate) const REPLY_PREVIEW_HEIGHT: f32 = 27.;

pub(crate) struct ReadRow {
    pub id: String,
    pub theme: Theme,
    pub name: String,
    pub avatar: Option<std::sync::Arc<RenderImage>>,
    pub header: bool,
    pub compact: bool,
    pub time: String,
    pub reply_count: usize,
    pub content: AnyElement,
    pub highlight: Option<AnyElement>,
}
pub(crate) fn row(props: ReadRow) -> Stateful<Div> {
    let ReadRow {
        id,
        theme,
        name,
        avatar,
        header,
        compact,
        time,
        reply_count,
        content,
        highlight,
    } = props;
    let metrics = row_metrics(compact, !header);
    let reply_offset = reply_count as f32 * REPLY_PREVIEW_HEIGHT;
    div()
        .id(ElementId::Name(format!("message-layout-{id}").into()))
        .w_full()
        .min_w_0()
        .px(px(15.))
        .mb(px(metrics.margin))
        .child(
            div()
                .id(ElementId::Name(format!("message-hover-{id}").into()))
                .group("timeline-message")
                .relative()
                .flex()
                .items_start()
                .w_full()
                .gap(px(11.))
                .px(px(9.))
                .pt(px(metrics.top))
                .pb(px(metrics.bottom))
                .rounded(px(theme.radius_sm))
                .hover(|style| style.bg(theme.message_hover))
                .children(highlight)
                .child(
                    div()
                        .w(px(36.))
                        .flex_shrink_0()
                        .pt(px(reply_offset))
                        .when(header, |slot| {
                            slot.child(stonks_ui::components::avatar(&theme, &name, 36., avatar))
                        })
                        .when(!header, |slot| {
                            slot.child(
                                div()
                                    .w_full()
                                    .pt(px(3.))
                                    .text_center()
                                    .text_size(px(theme.font_size_xs))
                                    .text_color(theme.text_muted)
                                    .opacity(0.)
                                    .group_hover("timeline-message", |style| style.opacity(1.))
                                    .child(time),
                            )
                        }),
                )
                .child(content),
        )
}
pub(crate) fn jump_visible(count: usize, nearby: bool, pinned: bool) -> bool {
    count > 0 && (nearby || !pinned)
}
pub(crate) fn within(distance: Option<f32>, threshold: f32, fallback: bool) -> bool {
    distance
        .filter(|distance| distance.is_finite())
        .map_or(fallback, |distance| distance.max(0.) < threshold)
}
pub(crate) fn jump_bottom(inset: f32) -> f32 {
    (inset - 10.).max(0.)
}
pub(crate) fn near_start(offset: f32) -> bool {
    offset.is_finite() && offset.max(0.) < NEAR_START_THRESHOLD
}
pub(crate) fn newest_visible<'a>(
    rows: &'a std::collections::HashMap<String, Bounds<Pixels>>,
    viewport: Bounds<Pixels>,
) -> Option<&'a str> {
    if viewport.size.height <= px(0.) {
        return None;
    }
    rows.iter()
        .filter(|(_, rect)| rect.bottom() > viewport.top() && rect.top() < viewport.bottom())
        .map(|(id, _)| id.as_str())
        .max()
}
pub(crate) fn distance_to_latest(list: &ListState, last_item: usize, inset: f32) -> Option<f32> {
    if list.is_following_tail() {
        return Some(0.);
    }
    let viewport = list.viewport_bounds();
    if viewport.size.height <= px(0.) {
        return None;
    }
    // GPUI's reported item bounds omit List padding; include the source's top
    // and bottom insets to recover the scroll-content bottom.
    list.bounds_for_item(last_item).map(|row| {
        f32::from((row.bottom() + px(TOP_PADDING + inset) - viewport.bottom()).max(px(0.)))
    })
}

pub(crate) fn intro(theme: &Theme, channel: &ChannelIntro, viewport_width: f32) -> Div {
    let name = channel.name.as_deref().unwrap_or_default();
    let description = channel
        .description
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| t_vars("timeline.beginningDescription", &[("channel", name)]));
    let font_size = (viewport_width * 0.03).clamp(25., 34.);
    div()
        .w_full()
        .min_w_0()
        .px(px(24.))
        .pt(px(22.))
        .pb(px(2.))
        .mb(px(8.))
        .child(
            div()
                .id("timeline-intro-title")
                .role(Role::Heading)
                .mb(px(5.))
                .text_size(px(font_size))
                .font_weight(FontWeight::MEDIUM)
                .line_height(relative(1.15))
                .text_color(theme.text)
                .child(name.to_owned()),
        )
        .child(
            div()
                .text_size(px(theme.font_size_sm))
                .line_height(relative(theme.line_height_normal))
                .text_color(theme.text_muted)
                .child(description),
        )
}

/// The host appends its shared Retry control for Failed. Plain state copy stays
/// independent of account/network state so the same surface can be previewed offline.
pub(crate) fn state(theme: &Theme, state: EmptyState, error: Option<&str>) -> Div {
    let mut view = div()
        .w_full()
        .min_h_full()
        .min_w_0()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .p(px(42.))
        .gap(px(theme.space4))
        .text_color(theme.text_muted)
        .text_center()
        .text_size(px(theme.font_size_md))
        .line_height(relative(1.5));
    if matches!(state, EmptyState::NoHistory | EmptyState::Empty) {
        view = view.child(
            div()
                .size(px(62.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(theme.radius_lg))
                .bg(theme.surface_raised)
                .text_color(theme.accent)
                .text_size(px(28.))
                .child("#"),
        );
    }
    let title = match state {
        EmptyState::Failed => Some("timeline.loadFailedTitle"),
        EmptyState::NoHistory => Some("timeline.historyUnavailableTitle"),
        EmptyState::Empty => Some("timeline.emptyTitle"),
        _ => None,
    };
    if let Some(title) = title {
        view = view.child(
            div()
                .id("timeline-state-title")
                .role(Role::Heading)
                .text_color(theme.text)
                .text_size(px(theme.font_size_xl))
                .font_weight(FontWeight::BOLD)
                .child(t(title)),
        );
    }
    let body = match state {
        EmptyState::NoChannel => t("shell.noChannel"),
        EmptyState::Loading => t("timeline.loadingMessages"),
        EmptyState::Failed => error.unwrap_or_default().to_owned(),
        EmptyState::NoHistory => t("timeline.historyUnavailableDescription"),
        EmptyState::Empty => t("timeline.emptyDescription"),
    };
    view.child(
        div()
            .max_w(px(430.))
            .line_height(relative(theme.line_height_normal))
            .child(body),
    )
}

pub(crate) fn error(theme: &Theme, message: &str) -> Stateful<Div> {
    div()
        .id("timeline-error")
        .role(Role::Alert)
        .mt(px(16.))
        .mx(px(24.))
        .text_center()
        .text_size(px(theme.font_size_sm))
        .text_color(theme.danger)
        .child(message.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        empty_state, first_day_divider, jump_bottom, jump_visible, show_intro, within, EmptyState,
        COMPOSER_INSET, NEAR_BOTTOM_THRESHOLD, PINNED_THRESHOLD,
    };
    #[test]
    fn read_markers_use_latest_actually_painted_partially_visible_row_even_mid_history() {
        use gpui_kit::{point, px, size, Bounds};
        let rect = |top, height| Bounds {
            origin: point(px(0.), px(top)),
            size: size(px(500.), px(height)),
        };
        let viewport = rect(100., 400.);
        let mut rows = std::collections::HashMap::from([
            ("01".into(), rect(20., 70.)),
            ("02".into(), rect(90., 30.)),
            ("03".into(), rect(480., 80.)),
            ("04".into(), rect(500., 40.)),
        ]);
        assert_eq!(super::newest_visible(&rows, viewport), Some("03"));
        rows.remove("03");
        assert_eq!(super::newest_visible(&rows, viewport), Some("02"));
        rows.clear();
        assert_eq!(super::newest_visible(&rows, viewport), None);
        rows.insert("05".into(), rect(150., 30.));
        assert_eq!(super::newest_visible(&rows, rect(100., 0.)), None);
    }
    #[test]
    fn selected_empty_channels_distinguish_loading_failure_and_permission_denial() {
        assert_eq!(
            empty_state(false, false, false, false),
            EmptyState::NoChannel
        );
        assert_eq!(empty_state(true, true, false, true), EmptyState::Loading);
        assert_eq!(empty_state(true, false, true, true), EmptyState::Failed);
        assert_eq!(
            empty_state(true, false, false, false),
            EmptyState::NoHistory
        );
        assert_eq!(empty_state(true, false, false, true), EmptyState::Empty);
    }
    #[test]
    fn history_intro_and_first_day_divider_follow_oldest_page_and_channel_name() {
        assert!(show_intro(false, false, 3, Some("General")));
        assert!(!first_day_divider(false, Some("General")));
        for (loading, more, count, name) in [
            (true, false, 3, Some("General")),
            (false, true, 3, Some("General")),
            (false, false, 0, Some("General")),
            (false, false, 3, None),
        ] {
            assert!(!show_intro(loading, more, count, name));
        }
        assert!(first_day_divider(true, Some("General")));
        assert!(first_day_divider(false, None));
    }
    #[test]
    fn source_thresholds_hide_jump_near_bottom_and_resume_new_messages_within_120px() {
        assert!(within(Some(79.), PINNED_THRESHOLD, false));
        assert!(!within(Some(80.), PINNED_THRESHOLD, true));
        assert!(within(Some(119.), NEAR_BOTTOM_THRESHOLD, false));
        assert!(!within(Some(120.), NEAR_BOTTOM_THRESHOLD, true));
        assert!(!jump_visible(0, false, false));
        assert!(jump_visible(3, false, false));
        assert!(!jump_visible(3, false, true));
        assert!(jump_visible(3, true, true));
        assert_eq!(jump_bottom(COMPOSER_INSET), 82.);
        assert!(super::near_start(139.));
        assert!(!super::near_start(140.));
    }
    #[test]
    fn author_grouping_excludes_system_boundaries_and_exact_seven_minute_gaps() {
        assert!(super::grouped(true, false, false, Some(419_999)));
        assert!(!super::grouped(true, false, false, Some(420_000)));
        assert!(!super::grouped(true, true, false, Some(1)));
        assert!(!super::grouped(true, false, true, Some(1)));
        assert!(!super::grouped(false, false, false, Some(1)));
        assert!(!super::grouped(true, false, false, None));
    }
    #[test]
    fn compact_row_spacing_preserves_avatar_column_and_overrides_grouped_padding() {
        assert_eq!(
            super::row_metrics(false, false),
            super::RowMetrics {
                top: 5.,
                bottom: 6.,
                margin: 3.
            }
        );
        assert_eq!(
            super::row_metrics(false, true),
            super::RowMetrics {
                top: 2.,
                bottom: 2.,
                margin: 3.
            }
        );
        assert_eq!(
            super::row_metrics(true, false),
            super::RowMetrics {
                top: 1.,
                bottom: 2.,
                margin: 0.
            }
        );
        assert_eq!(
            super::row_metrics(true, true),
            super::row_metrics(true, false)
        );
    }
}
