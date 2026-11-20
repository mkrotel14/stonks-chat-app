use super::*;
use stonks_ui::components::{permission_editor::PermissionEditor, UiAccordion};
pub(super) struct PermissionDraft {
    pub(super) value: PermissionOverride,
    pub(super) open: bool,
}

impl ChannelSettings {
    pub(super) fn permissions(
        &self,
        theme: Theme,
        content_width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut form = div()
            .flex()
            .flex_col()
            .min_w_0()
            .gap(px(theme.space4))
            .child(
                div()
                    .mt(px(theme.space1))
                    .mb(px(theme.space1))
                    .text_color(theme.text_muted)
                    .text_size(px(theme.font_size_md))
                    .child("Channel permissions override the server defaults for this channel."),
            );
        let mut roles = self.server.roles.iter().collect::<Vec<_>>();
        roles.sort_by(|(left_id, left), (right_id, right)| {
            left.rank
                .unwrap_or_default()
                .cmp(&right.rank.unwrap_or_default())
                .then_with(|| left_id.cmp(right_id))
        });
        let entries = std::iter::once((
            None,
            "Default permissions".to_owned(),
            "Applies to everyone",
            None,
        ))
        .chain(roles.into_iter().map(|(id, role)| {
            (
                Some(id.clone()),
                role.name.clone(),
                "Role override",
                role.colour.clone(),
            )
        }));
        for (role, title, subtitle, colour) in entries {
            let Some(draft) = self.permissions.get(&role) else {
                continue;
            };
            let id = role.as_deref().unwrap_or("default").to_owned();
            let toggle_role = role.clone();
            let edit_role = role.clone();
            let save_role = role.clone();
            let mut dot = div()
                .size(px(34.))
                .flex_shrink_0()
                .rounded(px(theme.radius_pill))
                .bg(theme.surface_interactive);
            if let Some(colour) = colour.as_deref().and_then(parse_colour) {
                dot = dot.bg(colour);
            }
            let entity = cx.entity().downgrade();
            let editor = PermissionEditor::new(
                format!("channel-permissions-{id}"),
                theme,
                draft.value.a,
                draft.value.d,
                move |allow, deny, _, cx| {
                    entity
                        .update(cx, |this, cx| {
                            if let Some(draft) = this.permissions.get_mut(&edit_role) {
                                draft.value = PermissionOverride { a: allow, d: deny };
                                this.notice = None;
                                cx.notify();
                            }
                        })
                        .ok();
                },
            )
            .disabled(self.busy || !self.access.manage_permissions)
            .compact(content_width - 18. <= 480.);
            form = form.child(
                UiAccordion::new(format!("channel-permission-{id}"), theme, title)
                    .subtitle(subtitle)
                    .leading(dot)
                    .open(draft.open)
                    .on_toggle(cx.listener(move |this, _, _, cx| {
                        if let Some(draft) = this.permissions.get_mut(&toggle_role) {
                            draft.open = !draft.open;
                            cx.notify();
                        }
                    }))
                    .child(
                        div().flex().flex_col().gap(px(7.)).child(editor).child(
                            div()
                                .flex()
                                .flex_wrap()
                                .justify_end()
                                .gap(px(theme.space2))
                                .mt(px(theme.space4))
                                .child(
                                    Button::new(
                                        ElementId::Name(
                                            format!("channel-permissions-save-{id}").into(),
                                        ),
                                        theme,
                                    )
                                    .label(if self.busy {
                                        "Saving…"
                                    } else {
                                        "Save permissions"
                                    })
                                    .disabled(self.busy || !self.access.manage_permissions)
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.save_permissions(save_role.clone(), cx)
                                        },
                                    )),
                                ),
                        ),
                    ),
            );
        }
        form.into_any_element()
    }
    fn save_permissions(&mut self, role: Option<String>, cx: &mut Context<Self>) {
        let Some(draft) = self.permissions.get(&role) else {
            return;
        };
        self.perform(Action::Permissions(role, draft.value.clone()), cx);
    }
}

fn parse_colour(value: &str) -> Option<Rgba> {
    let value = value.strip_prefix('#')?;
    if value.len() != 6 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(value, 16).ok().map(rgb)
}
