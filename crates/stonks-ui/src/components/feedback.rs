//! Port of `src/ui/primitives/Feedback.tsx`: persistent inline feedback
//! banner, urgency by tone.

use gpui_kit::*;

use crate::theme::Theme;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FeedbackTone {
    Error,
    Success,
}

#[derive(IntoElement)]
pub struct Feedback {
    theme: Theme,
    tone: FeedbackTone,
    message: SharedString,
}

impl Feedback {
    pub fn new(theme: Theme, tone: FeedbackTone, message: impl Into<SharedString>) -> Self {
        Self {
            theme,
            tone,
            message: message.into(),
        }
    }
}

impl RenderOnce for Feedback {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = &self.theme;
        div()
            .mt(px(theme.space2))
            .mb(px(theme.space4))
            .py(px(theme.space2))
            .px(px(theme.space3))
            .rounded(px(theme.radius_sm))
            .bg(theme.surface_raised)
            .text_size(px(theme.font_size_xs))
            .line_height(px(theme.font_size_xs * theme.line_height_normal))
            .text_color(match self.tone {
                FeedbackTone::Error => theme.danger,
                FeedbackTone::Success => theme.success,
            })
            .child(self.message)
    }
}
