use super::*;
use super::{actions::Mutation, extra::Dialog};
use std::collections::{HashMap, HashSet};
use stonks_core::connection::{ClientRole, PermissionOverride};
use stonks_ui::components::{ColorSwatch, PermissionEditor, Switch, UiAccordion, UiColorPicker};
const COLOURS: [&str; 10] = [
    "#5865f2", "#6952c7", "#3b8fc2", "#36a38c", "#d5a72e", "#df7047", "#d95763", "#c42f61",
    "#aa55c3", "#7a478e",
];
pub fn colour(value: &str) -> Option<Rgba> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    u32::from_str_radix(hex, 16).ok().map(rgb)
}
pub(super) struct RoleState {
    pub uploading_role: Option<String>,
    pub role_drafts: HashMap<String, RoleDraft>,
    pub expanded: HashSet<String>,
    pub role_name: Entity<InputState>,
}

impl ServerSettings {
    pub(super) fn render_roles(
        &mut self,
        theme: Theme,
        window: &mut Window,
        content_width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let create = settings_section(&theme, &t("serverSettings.roles"))
            .mt(px(theme.space2))
            .child(
                Button::new("create-server-role", theme)
                    .variant(ButtonVariant::SettingsRow)
                    .leading(
                        div()
                            .flex()
                            .w_full()
                            .items_center()
                            .gap(px(theme.space4))
                            .child(
                                div()
                                    .size(px(40.))
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(theme.radius_pill))
                                    .bg(theme.surface_interactive)
                                    .text_color(theme.accent)
                                    .child(Icon::Plus.render(theme.accent, 18.)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .min_w_0()
                                    .flex_1()
                                    .gap(px(theme.space1))
                                    .child(
                                        div()
                                            .text_size(px(theme.font_size_md))
                                            .line_height(relative(theme.line_height_compact))
                                            .font_weight(FontWeight::BOLD)
                                            .child(t("serverSettings.createRole")),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(theme.font_size_xs))
                                            .line_height(relative(theme.line_height_normal))
                                            .font_weight(FontWeight::NORMAL)
                                            .text_color(theme.text_muted)
                                            .child(t("serverSettings.createRoleHint")),
                                    ),
                            )
                            .child(Icon::ChevronRight.render(theme.text_muted, 16.)),
                    )
                    .disabled(!self.allowed(1 << 3) || self.busy)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_dialog(Dialog::CreateRole, window, cx)
                    })),
            );
        let mut rows = div().flex().flex_col().gap(px(theme.space4));
        let mut ids: Vec<_> = self.server.roles.keys().cloned().collect();
        ids.sort_by_key(|id| self.server.roles[id].rank.unwrap_or(0));
        ids.insert(0, String::new());
        for id in ids {
            let default = id.is_empty();
            let role = self.server.roles.get(&id);
            if !self.extra.roles.role_drafts.contains_key(&id) {
                self.extra.roles.role_drafts.insert(
                    id.clone(),
                    RoleDraft::new(&id, role, self.server.default_permissions, window, cx),
                );
            }
            let title = role
                .map(|role| role.name.clone())
                .unwrap_or_else(|| t("serverSettings.defaultPermissions"));
            let subtitle = t(if default {
                "serverSettings.everyMember"
            } else if role.is_some_and(|role| role.hoist) {
                "serverSettings.roleGrouping"
            } else {
                "serverSettings.roleUngrouped"
            });
            let leading = div().size(px(34.)).rounded(px(theme.radius_pill)).bg(role
                .and_then(|role| role.colour.as_deref())
                .and_then(colour)
                .unwrap_or(theme.surface_interactive));
            let leading = leading
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.accent)
                .child(if default { "≡" } else { "" });
            let open = self.extra.roles.expanded.contains(&id);
            let key = id.clone();
            let mut accordion = UiAccordion::new(
                ElementId::Name(format!("server-role-{id}").into()),
                theme,
                title,
            )
            .subtitle(subtitle)
            .leading(leading)
            .open(open)
            .on_toggle(cx.listener(move |this, _, _, cx| {
                if !this.extra.roles.expanded.insert(key.clone()) {
                    this.extra.roles.expanded.remove(&key);
                }
                cx.notify();
            }));
            if open {
                accordion = accordion.child(self.role_editor(&id, theme, content_width - 34., cx));
            }
            rows = rows.child(accordion);
        }
        div()
            .flex()
            .flex_col()
            .child(create)
            .child(settings_section(&theme, &t("serverSettings.serverRoles")).child(rows))
            .into_any_element()
    }
    fn role_editor(
        &self,
        id: &str,
        theme: Theme,
        content_width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let draft = &self.extra.roles.role_drafts[id];
        let default = id.is_empty();
        let can_role = self.allowed(1 << 3);
        let can_permissions = self.allowed(1 << 2);
        let disabled = self.busy;
        let mut body = div()
            .flex()
            .flex_col()
            .text_size(px(theme.font_size_md))
            .line_height(relative(1.5))
            .gap(px(theme.space4));
        if !default {
            body = body.child(input_field(
                &theme,
                &t("serverSettings.roleName"),
                "",
                SettingsInput::new(&draft.name, theme)
                    .label(t("serverSettings.roleName"))
                    .disabled(disabled || !can_role),
            ));
            let mut colours = div().flex().flex_wrap().gap(px(theme.space2)).child(
                UiColorPicker::new(&draft.picker, theme)
                    .label(t("serverSettings.customColour"))
                    .disabled(disabled || !can_role),
            );
            let current = draft.colour.read(cx).value().to_string();
            for (index, value) in COLOURS.into_iter().enumerate() {
                let key = id.to_owned();
                colours = colours.child(
                    ColorSwatch::new(
                        ElementId::Name(format!("role-colour-{id}-{index}").into()),
                        theme,
                        colour(value).unwrap_or(theme.accent),
                    )
                    .selected(current.eq_ignore_ascii_case(value))
                    .disabled(disabled || !can_role)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(draft) = this.extra.roles.role_drafts.get_mut(&key) {
                            draft.colour_changed = true;
                            draft
                                .colour
                                .update(cx, |input, cx| input.set_value(value, window, cx));
                            draft.picker.update(cx, |state, cx| {
                                state.set_value(
                                    colour(value).unwrap_or_else(|| rgb(0x5865f2)),
                                    window,
                                    cx,
                                )
                            });
                        }
                    })),
                );
            }
            body = body
                .child(settings_field(&theme, &t("serverSettings.roleColour"), "").child(colours));
            let url = draft.icon_url.clone().or_else(|| {
                self.server
                    .roles
                    .get(id)
                    .and_then(|role| role.icon.as_ref())
                    .and_then(|file| media::file_url(self.autumn.as_deref(), &file.tag, &file.id))
            });
            let image = draft.preview.clone().or_else(|| {
                self.images
                    .update(cx, |images, cx| images.get(url.as_deref(), 72, cx))
            });
            let key = id.to_owned();
            body = body.child(
                settings_field(&theme, &t("serverSettings.roleIcon"), "").child(
                    Button::new(ElementId::Name(format!("role-icon-{id}").into()), theme)
                        .variant(ButtonVariant::RoleIconUpload)
                        .tooltip(t("serverSettings.changeIcon"))
                        .disabled(disabled || !can_role)
                        .leading(
                            div()
                                .relative()
                                .size(px(72.))
                                .child(avatar(&theme, "+", 72., image))
                                .child(
                                    div()
                                        .absolute()
                                        .bottom_0()
                                        .left_0()
                                        .right_0()
                                        .py(px(6.))
                                        .px(px(2.))
                                        .text_center()
                                        .bg(theme.surface_raised)
                                        .text_color(theme.text)
                                        .text_size(px(theme.font_size_xs))
                                        .child(t(
                                            if self.extra.roles.uploading_role.as_deref()
                                                == Some(id)
                                            {
                                                "serverSettings.uploading"
                                            } else {
                                                "serverSettings.changeIcon"
                                            },
                                        )),
                                ),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.pick_icon(Some(key.clone()), window, cx)
                        })),
                ),
            );
            if let Some(error) = &draft.upload_error {
                body = body.child(Feedback::new(theme, FeedbackTone::Error, error.clone()));
            }
            let key = id.to_owned();
            body = body.child(settings_row(
                &theme,
                &t("serverSettings.displayRole"),
                &t("serverSettings.displayRoleHint"),
                Switch::new(ElementId::Name(format!("role-hoist-{id}").into()), theme)
                    .label(t("serverSettings.displayRole"))
                    .checked(draft.hoist)
                    .disabled(disabled || !can_role)
                    .on_change(cx.listener(move |this, value, _, cx| {
                        if let Some(draft) = this.extra.roles.role_drafts.get_mut(&key) {
                            draft.hoist = *value;
                            cx.notify();
                        }
                    })),
            ));
        }
        let key = id.to_owned();
        let weak = cx.entity().downgrade();
        body = body.child(
            PermissionEditor::new(
                format!("role-permissions-{id}"),
                theme,
                draft.permissions.a,
                draft.permissions.d,
                move |a, d, _, cx| {
                    weak.update(cx, |this, cx| {
                        if let Some(draft) = this.extra.roles.role_drafts.get_mut(&key) {
                            draft.permissions.a = a;
                            draft.permissions.d = d;
                            cx.notify();
                        }
                    })
                    .ok();
                },
            )
            .binary(default)
            .compact(content_width <= 480.)
            .disabled(disabled || !can_permissions),
        );
        let mut actions = div().flex().flex_wrap().gap(px(theme.space2)).justify_end();
        if !default {
            let key = id.to_owned();
            actions = actions.child(
                Button::new(ElementId::Name(format!("copy-role-{id}").into()), theme)
                    .variant(ButtonVariant::Secondary)
                    .label(t("serverSettings.copyRoleId"))
                    .disabled(disabled)
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(key.clone()))
                    }),
            );
        }
        let key = id.to_owned();
        actions = actions.child(
            Button::new(ElementId::Name(format!("save-role-{id}").into()), theme)
                .label(t(if self.busy {
                    "serverSettings.saving"
                } else {
                    "serverSettings.save"
                }))
                .disabled(
                    disabled
                        || if default {
                            !can_permissions
                        } else {
                            (!can_role && !can_permissions)
                                || (can_role
                                    && (draft.name.read(cx).value().trim().is_empty()
                                        || (draft.colour_changed
                                            && colour(&draft.colour.read(cx).value()).is_none())))
                        },
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.save_role_draft(key.clone(), window, cx)
                })),
        );
        if !default {
            let key = id.to_owned();
            let name = self
                .server
                .roles
                .get(id)
                .map(|role| role.name.clone())
                .unwrap_or_else(|| draft.name.read(cx).value().to_string());
            actions = actions.child(
                Button::new(ElementId::Name(format!("delete-role-{id}").into()), theme)
                    .variant(ButtonVariant::Danger)
                    .label(t("serverSettings.deleteRole"))
                    .disabled(disabled || !can_role)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_dialog(Dialog::DeleteRole(key.clone(), name.clone()), window, cx)
                    })),
            );
        }
        body.child(actions).into_any_element()
    }
    fn save_role_draft(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.extra.roles.role_drafts.get(&id) else {
            return;
        };
        let permissions = draft.permissions.clone();
        let mutation = if id.is_empty() {
            Mutation::DefaultPermissions(permissions)
        } else if !self.allowed(1 << 3) {
            Mutation::RolePermissions(id, permissions)
        } else {
            Mutation::SaveRole(
                id,
                service::RolePatch {
                    name: draft.name.read(cx).value().to_string(),
                    colour: draft
                        .colour_changed
                        .then(|| draft.colour.read(cx).value().to_string()),
                    hoist: draft.hoist,
                    icon: draft.icon.clone(),
                },
                self.allowed(1 << 2).then_some(permissions),
            )
        };
        self.mutate(mutation, window, cx);
    }
}

pub struct RoleDraft {
    pub name: Entity<InputState>,
    pub colour: Entity<InputState>,
    pub colour_changed: bool,
    pub picker: Entity<stonks_ui::components::UiColorPickerState>,
    pub hoist: bool,
    pub permissions: PermissionOverride,
    pub icon: Option<String>,
    pub icon_url: Option<String>,
    pub preview: Option<Arc<RenderImage>>,
    pub upload_error: Option<String>,
    pub _subscriptions: Vec<Subscription>,
}
impl RoleDraft {
    pub fn new(
        id: &str,
        role: Option<&ClientRole>,
        defaults: u64,
        window: &mut Window,
        cx: &mut Context<ServerSettings>,
    ) -> Self {
        let name = cx
            .new(|cx| InputState::new(window, cx).validate(|value, _| value.chars().count() <= 32));
        name.update(cx, |input, cx| {
            input.set_value(
                role.map(|role| role.name.clone()).unwrap_or_default(),
                window,
                cx,
            )
        });
        let colour = cx.new(|cx| InputState::new(window, cx).validate(|value, _| value.len() <= 7));
        colour.update(cx, |input, cx| {
            input.set_value(
                role.and_then(|role| role.colour.clone())
                    .unwrap_or_else(|| "#5865f2".into()),
                window,
                cx,
            )
        });
        let picker = cx.new(|cx| stonks_ui::components::UiColorPickerState::new(window, cx));
        let initial = self::colour(&colour.read(cx).value()).unwrap_or_else(|| rgb(0x5865f2));
        picker.update(cx, |state, cx| state.set_value(initial, window, cx));
        let mut subscriptions = vec![
            cx.observe(&name, |_, _, cx| cx.notify()),
            cx.observe(&colour, |_, _, cx| cx.notify()),
        ];
        let input = colour.clone();
        let key = id.to_owned();
        subscriptions.push(
            cx.subscribe_in(&picker, window, move |this, _, event, window, cx| {
                if let stonks_ui::components::UiColorPickerEvent::Change(Some(value)) = event {
                    if let Some(draft) = this.extra.roles.role_drafts.get_mut(&key) {
                        draft.colour_changed = true;
                    }
                    let value = Rgba::from(*value);
                    let hex = format!(
                        "#{:02x}{:02x}{:02x}",
                        (value.r * 255.).round() as u8,
                        (value.g * 255.).round() as u8,
                        (value.b * 255.).round() as u8
                    );
                    input.update(cx, |input, cx| input.set_value(hex, window, cx));
                    cx.notify();
                }
            }),
        );
        Self {
            name,
            colour,
            colour_changed: role.is_some_and(|role| role.colour.is_none()),
            picker,
            hoist: role.is_some_and(|role| role.hoist),
            permissions: role
                .map(|role| role.permissions.clone())
                .unwrap_or(PermissionOverride { a: defaults, d: 0 }),
            icon: None,
            icon_url: None,
            preview: None,
            upload_error: None,
            _subscriptions: subscriptions,
        }
    }
}
/// Keep locally edited bits; accept remote changes to untouched permissions.
/// Keep locally edited bits; accept remote changes to untouched permissions.
pub(super) fn merge_permissions(
    local: &stonks_core::connection::PermissionOverride,
    before: &stonks_core::connection::PermissionOverride,
    remote: &stonks_core::connection::PermissionOverride,
    binary: bool,
) -> stonks_core::connection::PermissionOverride {
    let dirty = (local.a ^ before.a) | (local.d ^ before.d);
    stonks_core::connection::PermissionOverride {
        a: (remote.a & !dirty) | (local.a & dirty),
        d: if binary {
            0
        } else {
            (remote.d & !dirty) | (local.d & dirty)
        },
    }
}

#[cfg(test)]
mod tests {
    use super::merge_permissions;
    #[test]
    fn remote_permissions_merge_without_discarding_local_edits() {
        use stonks_core::connection::PermissionOverride as P;
        let before = P { a: 1 << 22, d: 0 };
        let local = P {
            a: (1 << 22) | (1 << 41),
            d: 1 << 33,
        };
        let remote = P {
            a: (1 << 22) | (1 << 29),
            d: 1 << 38,
        };
        let merged = merge_permissions(&local, &before, &remote, false);
        assert_eq!(merged.a, (1 << 22) | (1 << 29) | (1 << 41));
        assert_eq!(merged.d, (1 << 33) | (1 << 38));
        let binary = merge_permissions(
            &P { a: 0, d: 0 },
            &P { a: 1 << 22, d: 0 },
            &P {
                a: (1 << 22) | (1 << 29),
                d: 0,
            },
            true,
        );
        assert_eq!(binary.a, 1 << 29);
        assert_eq!(binary.d, 0);
    }
}

impl ServerSettings {
    pub(super) fn role_dialog(
        &self,
        dialog: &Dialog,
        theme: Theme,
        cx: &mut Context<Self>,
        mut content: Div,
        mut footer: Div,
    ) -> (Div, Div) {
        match dialog {
            Dialog::CreateRole => {
                content = content.child(input_field(
                    &theme,
                    &t("serverSettings.roleName"),
                    &format!(
                        "{}/32",
                        self.extra.roles.role_name.read(cx).value().chars().count()
                    ),
                    SettingsInput::new(&self.extra.roles.role_name, theme)
                        .label(t("serverSettings.roleName"))
                        .disabled(self.busy),
                ));
                footer = footer.child(self.dialog_cancel(theme, cx)).child(
                    Button::new("create-role-confirm", theme)
                        .label(t(if self.busy {
                            "serverSettings.creating"
                        } else {
                            "serverSettings.createRole"
                        }))
                        .disabled(
                            self.busy
                                || !self.allowed(1 << 3)
                                || self
                                    .extra
                                    .roles
                                    .role_name
                                    .read(cx)
                                    .value()
                                    .trim()
                                    .is_empty(),
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.mutate(
                                Mutation::CreateRole(
                                    this.extra.roles.role_name.read(cx).value().to_string(),
                                ),
                                window,
                                cx,
                            )
                        })),
                );
            }
            Dialog::DeleteRole(id, _) => {
                let id = id.clone();
                footer = footer.child(self.dialog_cancel(theme, cx)).child(
                    Button::new("delete-role-confirm", theme)
                        .variant(ButtonVariant::Danger)
                        .label(t("serverSettings.deleteRole"))
                        .disabled(self.busy || !self.allowed(1 << 3))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.mutate(Mutation::DeleteRole(id.clone()), window, cx)
                        })),
                );
            }
            _ => unreachable!("dialog routed to its workflow"),
        }
        (content, footer)
    }
}
