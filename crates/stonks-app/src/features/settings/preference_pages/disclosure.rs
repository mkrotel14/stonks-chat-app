use super::*;
use std::rc::Rc;
use stonks_ui::icons::Icon;

#[derive(Default)]
pub(super) struct Disclosure {
    open: bool,
    trigger: Option<FocusHandle>,
    choices: Vec<FocusHandle>,
}

pub(super) struct Choice {
    pub label: String,
    pub description: Option<String>,
    pub change: Option<Change>,
}

pub(super) fn icon(theme: &Theme, icon: Icon) -> Div {
    div()
        .size(px(34.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme.radius_pill))
        .bg(theme.surface_interactive)
        .child(icon.render_stroke(theme.accent, 18., 1.6))
}

// Reuse the same disclosure and radio buttons as the native camera setup.
#[allow(clippy::too_many_arguments)]
pub(super) fn render(
    field: &mut Disclosure,
    access: fn(&mut State) -> &mut Disclosure,
    id: &'static str,
    theme: &Theme,
    title: String,
    leading: Option<Icon>,
    options: Vec<Choice>,
    selected: usize,
    disabled: bool,
    cx: &Context<State>,
) -> UiAccordion {
    let trigger = field
        .trigger
        .get_or_insert_with(|| cx.focus_handle())
        .clone();
    let weak = cx.entity().downgrade();
    let mut accordion = UiAccordion::new(id, *theme, title)
        .subtitle(options[selected].label.clone())
        .open(field.open)
        .track_focus(&trigger)
        .on_toggle(move |_, _, cx| {
            let _ = weak.update(cx, |state, cx| {
                let field = access(state);
                field.open = !field.open;
                cx.notify();
            });
        });
    if let Some(leading) = leading {
        accordion = accordion.leading(icon(theme, leading));
    }
    if !field.open {
        return accordion;
    }

    field
        .choices
        .resize_with(options.len(), || cx.focus_handle());
    let options = Rc::new(options);
    let focuses = Rc::new(field.choices.clone());
    let list = div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .children(options.iter().enumerate().map(|(index, option)| {
            let mut copy = div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(theme.space1))
                .child(
                    div()
                        .truncate()
                        .text_size(px(theme.font_size_sm))
                        .child(option.label.clone()),
                );
            if let Some(description) = &option.description {
                copy = copy.child(
                    div()
                        .text_size(px(theme.font_size_xs))
                        .font_weight(FontWeight::NORMAL)
                        .text_color(theme.text_muted)
                        .child(description.clone()),
                );
            }
            let weak = cx.entity().downgrade();
            let change = option.change.clone();
            let unavailable = disabled || change.is_none();
            let focus = trigger.clone();
            let button = Button::new((id, index), *theme)
                .variant(ButtonVariant::DisclosureChoice)
                .track_focus(&focuses[index])
                .accessibility_label(option.label.clone())
                .leading(copy)
                .selected(index == selected)
                .disabled(unavailable)
                .on_click(move |_, window, cx| {
                    if let Some(change) = change.clone() {
                        let _ = weak.update(cx, |state, cx| {
                            access(state).open = false;
                            state.save(change, window, cx);
                            focus.focus(window, cx);
                        });
                    }
                });
            let weak = cx.entity().downgrade();
            let options = options.clone();
            let focuses = focuses.clone();
            let trigger = trigger.clone();
            div()
                .id((id, index + options.len()))
                .on_key_down(move |event, window, cx| {
                    let key = event.keystroke.key.as_str();
                    if key == "escape" {
                        cx.stop_propagation();
                        let _ = weak.update(cx, |state, cx| {
                            access(state).open = false;
                            trigger.focus(window, cx);
                            cx.notify();
                        });
                        return;
                    }
                    if unavailable {
                        return;
                    }
                    let len = options.len();
                    let next = match key {
                        "down" | "right" => (1..len)
                            .map(|step| (index + step) % len)
                            .find(|&i| options[i].change.is_some()),
                        "up" | "left" => (1..len)
                            .map(|step| (index + len - step) % len)
                            .find(|&i| options[i].change.is_some()),
                        "home" => options.iter().position(|option| option.change.is_some()),
                        "end" => options.iter().rposition(|option| option.change.is_some()),
                        _ => return,
                    };
                    cx.stop_propagation();
                    if let Some(next) = next.filter(|&next| next != index) {
                        if let Some(change) = options[next].change.clone() {
                            let _ = weak.update(cx, |state, cx| {
                                state.save(change, window, cx);
                                focuses[next].focus(window, cx);
                            });
                        }
                    }
                })
                .child(button)
        }));
    accordion.child(list)
}
