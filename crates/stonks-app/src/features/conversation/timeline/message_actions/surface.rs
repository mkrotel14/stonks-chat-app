//! UI only: the timeline owns eligibility, channel lifetimes, and mutations.
//!
//! Keep one entity per timeline. Observe it to invalidate the parent render,
//! and subscribe to `ActionRequest` for mutations followed by `finish`. Use
//! `editor` inside the target row and `overlay` once at window-overlay level.
//! Clear on channel change, target invalidation, or scrolling the reaction
//! anchor away (the anchor is a snapshot in window coordinates).
use std::rc::Rc;

use gpui_kit::component::input::{InputEvent, TextareaState};
use gpui_kit::*;
use stonks_ui::{
    components::{
        Button, ButtonVariant, EmojiPickerEvent, Feedback, FeedbackTone, SettingsInput,
        UiCloseButton, UiDialog, UiEmojiPicker,
    },
    i18n::t,
    Theme,
};

#[derive(Clone, Debug)]
pub(super) enum Submission {
    React { value: String, custom: bool },
    Edit { draft: String },
    Delete,
    Report { details: String },
}

pub(in super::super) struct ActionRequest {
    pub ticket: u64,
    pub message_id: String,
    pub(super) submission: Submission,
}

pub(super) enum OpenAction {
    React(Bounds<Pixels>),
    Edit(String),
    Delete(DeletePreview),
    Report,
}

pub(super) struct DeletePreview {
    pub author: SharedString,
    pub timestamp: SharedString,
    /// Message content or the attachment filename fallback.
    pub body: SharedString,
    pub avatar: Option<Rc<dyn Fn(&mut App) -> AnyElement>>,
}

enum Surface {
    Reaction(Bounds<Pixels>),
    Edit(Entity<TextareaState>),
    Delete(DeletePreview),
    Report(Entity<TextareaState>),
}

struct Active {
    message_id: String,
    surface: Surface,
    return_focus: Option<FocusHandle>,
    pending: Option<u64>,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

pub(in super::super) struct MessageActionUi {
    picker: Entity<UiEmojiPicker>,
    active: Option<Active>,
    dialog_focus: FocusHandle,
    next_ticket: u64,
    _picker_subscription: Subscription,
}

impl EventEmitter<ActionRequest> for MessageActionUi {}

impl MessageActionUi {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let picker = cx.new(|cx| UiEmojiPicker::new(window, cx));
        let subscription = cx.subscribe(&picker, |this, _, event, cx| {
            if matches!(
                this.active.as_ref().map(|a| &a.surface),
                Some(Surface::Reaction(_))
            ) {
                let EmojiPickerEvent::Select { value, custom } = event;
                this.submit(
                    Submission::React {
                        value: value.clone(),
                        custom: *custom,
                    },
                    cx,
                );
            }
        });
        Self {
            picker,
            active: None,
            dialog_focus: cx.focus_handle(),
            next_ticket: 0,
            _picker_subscription: subscription,
        }
    }

    /// Configure this dedicated instance with the composer's scoped catalog and image
    /// provider; never share the composer's entity (its selection inserts text).
    pub fn picker(&self) -> &Entity<UiEmojiPicker> {
        &self.picker
    }

    pub fn target(&self) -> Option<&str> {
        self.active
            .as_ref()
            .map(|active| active.message_id.as_str())
    }

    pub fn action(&self) -> Option<super::policy::MessageAction> {
        use super::policy::MessageAction;
        Some(match &self.active.as_ref()?.surface {
            Surface::Reaction(_) => MessageAction::React,
            Surface::Edit(_) => MessageAction::Edit,
            Surface::Delete(_) => MessageAction::Delete,
            Surface::Report(_) => MessageAction::Report,
        })
    }

    pub fn editing(&self, message_id: &str) -> bool {
        self.active
            .as_ref()
            .is_some_and(|a| a.message_id == message_id && matches!(a.surface, Surface::Edit(_)))
    }

    pub fn reacting(&self, message_id: &str) -> bool {
        self.active.as_ref().is_some_and(|a| {
            a.message_id == message_id && matches!(a.surface, Surface::Reaction(_))
        })
    }

    /// Open only after rechecking eligibility and closing any context menu.
    /// A React action for the same row toggles the picker.
    pub(super) fn open(
        &mut self,
        message_id: String,
        action: OpenAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy() {
            return;
        }
        if self.reacting(&message_id) && matches!(action, OpenAction::React(_)) {
            self.clear(window, cx);
            return;
        }
        self.clear(window, cx);
        let return_focus = window.focused(cx);
        let mut subscriptions = Vec::new();
        let surface = match action {
            OpenAction::React(anchor) => {
                self.picker.update(cx, |picker, cx| {
                    picker.reset(window, cx);
                    picker.focus_search(window, cx);
                });
                Surface::Reaction(anchor)
            }
            OpenAction::Edit(draft) => {
                let input = cx.new(|cx| {
                    let mut input = TextareaState::new(window, cx);
                    input.set_value(draft, window, cx);
                    input.focus(window, cx);
                    input
                });
                subscriptions.push(cx.subscribe(&input, |this, _, event, cx| match event {
                    InputEvent::PressEnter { shift: false, .. } => this.submit_form(cx),
                    InputEvent::Change => cx.notify(),
                    _ => {}
                }));
                Surface::Edit(input)
            }
            OpenAction::Delete(preview) => {
                self.dialog_focus.focus(window, cx);
                Surface::Delete(preview)
            }
            OpenAction::Report => {
                let details = cx.new(|cx| TextareaState::new(window, cx));
                subscriptions.push(cx.subscribe(&details, |_, _, _: &InputEvent, cx| cx.notify()));
                self.dialog_focus.focus(window, cx);
                Surface::Report(details)
            }
        };
        self.active = Some(Active {
            message_id,
            surface,
            return_focus,
            pending: None,
            error: None,
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    /// Also call on channel/target invalidation, even when a request is pending.
    pub fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(active) = self.active.take() {
            if let Some(focus) = active.return_focus {
                focus.focus(window, cx);
            }
            self.picker
                .update(cx, |picker, cx| picker.set_disabled(false, cx));
            cx.notify();
        }
    }

    /// Ignore late completions after a target change; a failed request keeps the form open.
    pub fn finish(
        &mut self,
        ticket: u64,
        result: Result<(), SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(active) = self.active.as_mut().filter(|a| a.pending == Some(ticket)) else {
            return;
        };
        match result {
            Ok(()) => self.clear(window, cx),
            Err(error) => {
                active.pending = None;
                active.error = Some(error);
                self.picker
                    .update(cx, |picker, cx| picker.set_disabled(false, cx));
                cx.notify();
            }
        }
    }

    fn busy(&self) -> bool {
        self.active.as_ref().is_some_and(|a| a.pending.is_some())
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.busy() {
            self.clear(window, cx);
        }
    }

    fn form_submission(&self, cx: &App) -> Option<Submission> {
        match &self.active.as_ref()?.surface {
            Surface::Edit(input) => edit_submission(&input.read(cx).value()),
            Surface::Delete(_) => Some(Submission::Delete),
            Surface::Report(details) => Some(Submission::Report {
                details: report_details(&details.read(cx).value())?,
            }),
            Surface::Reaction(_) => None,
        }
    }

    fn submit_form(&mut self, cx: &mut Context<Self>) {
        if let Some(submission) = self.form_submission(cx) {
            self.submit(submission, cx);
        }
    }

    fn submit(&mut self, submission: Submission, cx: &mut Context<Self>) {
        let Some(active) = self.active.as_mut().filter(|a| a.pending.is_none()) else {
            return;
        };
        self.next_ticket += 1;
        active.pending = Some(self.next_ticket);
        active.error = None;
        if matches!(submission, Submission::React { .. }) {
            self.picker
                .update(cx, |picker, cx| picker.set_disabled(true, cx));
        }
        cx.emit(ActionRequest {
            ticket: self.next_ticket,
            message_id: active.message_id.clone(),
            submission,
        });
        cx.notify();
    }

    fn footer(
        &self,
        theme: Theme,
        label: String,
        variant: ButtonVariant,
        inline: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut footer = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_end()
            .gap(px(theme.space2));
        if inline {
            footer = footer.child(
                div()
                    .flex_1()
                    .text_size(px(theme.font_size_xs))
                    .text_color(theme.text_muted)
                    .child(t("messages.editHint")),
            );
        }
        footer
            .child(
                Button::new("action-cancel", theme)
                    .variant(if inline {
                        ButtonVariant::Quiet
                    } else {
                        ButtonVariant::Secondary
                    })
                    .label(t("common.cancel"))
                    .disabled(self.busy())
                    .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx))),
            )
            .child(
                Button::new("action-submit", theme)
                    .variant(variant)
                    .label(label)
                    .disabled(self.busy() || self.form_submission(cx).is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.submit_form(cx))),
            )
    }

    /// Mount in place of the message body for this row; keep attachments visible.
    pub fn editor(&self, message_id: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        let active = self
            .active
            .as_ref()
            .filter(|a| a.message_id == message_id)?;
        let Surface::Edit(input) = &active.surface else {
            return None;
        };
        let theme = Theme::current(cx);
        Some(
            div()
                .id("message-inline-editor")
                .w_full()
                .min_w_0()
                .mt(px(theme.space1))
                .flex()
                .flex_col()
                .gap(px(theme.space2))
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" {
                        this.cancel(window, cx);
                        window.prevent_default();
                        cx.stop_propagation();
                    }
                }))
                .child(
                    SettingsInput::textarea(input, theme)
                        .label("Edit message")
                        .disabled(self.busy()),
                )
                .children(feedback(active, theme))
                .child(self.footer(theme, t("common.save"), ButtonVariant::Primary, true, cx))
                .into_any_element(),
        )
    }

    /// Mount once in the window-sized overlay layer, not inside a clipped list.
    pub fn overlay(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = Theme::current(cx);
        let active = self.active.as_ref()?;
        let mut content = div().flex().flex_col().gap(px(theme.space4));
        let (title, submit, variant) = match &active.surface {
            Surface::Edit(_) => return None,
            Surface::Reaction(anchor) => return Some(self.reaction_popup(*anchor, window, cx)),
            Surface::Delete(preview) => {
                content = content.child(t("messages.deleteConfirm")).child(
                    div()
                        .flex()
                        .gap(px(theme.space3))
                        .rounded(px(theme.radius_sm))
                        .p(px(theme.space3))
                        .bg(theme.surface)
                        .children(preview.avatar.as_ref().map(|avatar| avatar(cx)))
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .flex()
                                        .flex_wrap()
                                        .items_center()
                                        .gap(px(theme.space2))
                                        .child(
                                            div()
                                                .font_weight(FontWeight::BOLD)
                                                .child(preview.author.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(theme.font_size_xs))
                                                .text_color(theme.text_muted)
                                                .child(preview.timestamp.clone()),
                                        ),
                                )
                                .child(preview.body.clone()),
                        ),
                );
                (
                    t("messages.delete"),
                    t(if active.pending.is_some() {
                        "messages.deleting"
                    } else {
                        "common.delete"
                    }),
                    ButtonVariant::Danger,
                )
            }
            Surface::Report(details) => {
                let busy = active.pending.is_some();
                content = content.child(field(
                    theme,
                    "Details (optional, up to 1,000 characters)",
                    SettingsInput::textarea(details, theme)
                        .label("Details")
                        .disabled(busy),
                ));
                (
                    "Report message".into(),
                    "Report".into(),
                    ButtonVariant::Primary,
                )
            }
        };
        content = content.children(feedback(active, theme));
        Some(
            UiDialog::new("message-action-dialog", theme, title)
                .focus(&self.dialog_focus)
                .busy(self.busy())
                .on_close(cx.listener(|this, _, window, cx| this.cancel(window, cx)))
                .child(content.child(self.footer(theme, submit, variant, false, cx)))
                .into_any_element(),
        )
    }

    fn reaction_popup(
        &self,
        anchor: Bounds<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::current(cx);
        let (origin, size) = picker_geometry(anchor, window.viewport_size());
        let panel = div()
            .id("message-reaction-picker")
            .role(Role::Dialog)
            .aria_label("Add reaction")
            .occlude()
            .w(size.width)
            .h(size.height)
            .p(px(theme.space3))
            .min_h_0()
            .overflow_y_scroll()
            .bg(theme.surface_raised)
            .text_color(theme.text)
            .border_1()
            .border_color(theme.outline)
            .rounded(px(theme.radius_lg))
            .shadow(vec![BoxShadow {
                color: theme.shadow_md.color.into(),
                offset: point(px(0.), px(theme.shadow_md.dy)),
                blur_radius: px(theme.shadow_md.blur),
                spread_radius: px(0.),
                inset: false,
            }])
            .on_mouse_down_out(
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    // Let the toolbar's click toggle the picker instead of reopening it.
                    if !anchor.contains(&event.position) {
                        this.cancel(window, cx);
                    }
                }),
            )
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape"
                    && !gpui_kit::base::GlobalState::is_in_deferred_context(cx)
                {
                    this.cancel(window, cx);
                    window.prevent_default();
                    cx.stop_propagation();
                }
            }))
            .child(
                div().flex().justify_end().child(
                    UiCloseButton::new("message-reaction-close", theme)
                        .disabled(self.busy())
                        .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx))),
                ),
            )
            .children(self.active.as_ref().and_then(|a| feedback(a, theme)))
            .child(self.picker.clone());
        deferred(
            anchored()
                .anchor(Anchor::TopLeft)
                .position_mode(AnchoredPositionMode::Window)
                .position(origin)
                .child(panel),
        )
        .with_priority(1)
        .into_any_element()
    }
}

fn feedback(active: &Active, theme: Theme) -> Option<Feedback> {
    active
        .error
        .clone()
        .map(|error| Feedback::new(theme, FeedbackTone::Error, error))
}

fn field(theme: Theme, label: &'static str, control: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(theme.space2))
        .child(
            div()
                .text_size(px(theme.font_size_sm))
                .font_weight(FontWeight::SEMIBOLD)
                .child(label),
        )
        .child(control)
}

fn edit_submission(draft: &str) -> Option<Submission> {
    let draft = draft.trim_end();
    (!draft.is_empty()).then(|| Submission::Edit {
        draft: draft.to_owned(),
    })
}

fn report_details(details: &str) -> Option<String> {
    let details = details.trim();
    (details.chars().count() <= 1_000).then(|| details.to_owned())
}

fn picker_geometry(
    anchor: Bounds<Pixels>,
    viewport: Size<Pixels>,
) -> (Point<Pixels>, Size<Pixels>) {
    let margin = px(12.).min(viewport.width / 4.).min(viewport.height / 4.);
    let gap = px(8.);
    let width = px(340.).min(viewport.width - margin * 2.);
    let height = px(520.).min(viewport.height - margin * 2.);
    let right = viewport.width - margin;
    let bottom = viewport.height - margin;
    let above = (anchor.top() - gap).clamp(margin, bottom);
    let below = (anchor.bottom() + gap).clamp(margin, bottom);
    let y = if below + height <= bottom {
        below
    } else if above - height >= margin {
        above - height
    } else if above - margin > bottom - below {
        margin
    } else {
        bottom - height
    };
    (
        point((anchor.right() - width).clamp(margin, right - width), y),
        size(width, height),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn picker_flips_and_clamps_at_viewport_edges() {
        let viewport = size(px(800.), px(900.));
        let top = Bounds::new(point(px(0.), px(10.)), size(px(30.), px(30.)));
        let bottom = Bounds::new(point(px(790.), px(860.)), size(px(30.), px(30.)));
        assert_eq!(picker_geometry(top, viewport).0, point(px(12.), px(48.)));
        assert_eq!(
            picker_geometry(bottom, viewport).0,
            point(px(448.), px(332.))
        );
        let tiny = size(px(120.), px(100.));
        let (origin, size) = picker_geometry(bottom, tiny);
        assert_eq!(origin, point(px(12.), px(12.)));
        assert_eq!(size, gpui_kit::size(px(96.), px(76.)));
    }

    #[::core::prelude::v1::test]
    fn validates_form_payloads() {
        assert!(edit_submission(" \n").is_none());
        assert!(
            matches!(edit_submission("  hi\n"), Some(Submission::Edit { draft }) if draft == "  hi")
        );
        assert_eq!(report_details("  ").as_deref(), Some(""));
        assert!(report_details(&"é".repeat(1_000)).is_some());
        assert!(report_details(&"é".repeat(1_001)).is_none());
    }
}
