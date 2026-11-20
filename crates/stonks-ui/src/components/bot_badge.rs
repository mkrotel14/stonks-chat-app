//! Source BotBadge.tsx/BotBadge.stylex.ts geometry and semantic colors.
use crate::Theme;
use gpui_kit::*;

#[derive(IntoElement)]
pub struct BotBadge {
    theme: Theme,
}
impl BotBadge {
    /// Render only for bot accounts, matching the source conditional badge.
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }
}
impl RenderOnce for BotBadge {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .id("bot-badge")
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .min_w(px(27.))
            .h(px(16.))
            .ml(px(6.))
            .px(px(5.))
            .rounded(px(self.theme.radius_sm))
            .text_color(self.theme.accent_text)
            .bg(self.theme.accent)
            .text_size(px(8.))
            .font_weight(FontWeight::BLACK)
            .line_height(relative(1.))
            .aria_label("Bot account")
            .tooltip(|window, cx| {
                gpui_kit::component::tooltip::Tooltip::new("Bot account").build(window, cx)
            })
            // GPUI has no letter-spacing style: three glyphs with the source
            // 0.05em spacing preserve its compact treatment at 8px.
            .child(div().flex().gap(px(0.4)).child("B").child("O").child("T"))
    }
}
