//! ContextMenu.tsx interaction and measured contextMenu.stylex geometry.
use crate::{icons::Icon, Theme};
use gpui_kit::*;
use std::collections::HashMap;
mod navigation;
use navigation::{key_intent, navigation_panel, next_enabled, KeyIntent};

#[derive(Clone, Debug)]
pub enum MenuEntry {
    Item {
        id: String,
        label: String,
        danger: bool,
        disabled: bool,
        checked: bool,
    },
    Submenu {
        label: String,
        disabled: bool,
        items: Vec<MenuEntry>,
    },
    Separator,
}
impl MenuEntry {
    pub fn action(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self::Item {
            id: id.into(),
            label: label.into(),
            danger: false,
            disabled: false,
            checked: false,
        }
    }
    pub fn danger(self) -> Self {
        match self {
            Self::Item {
                id,
                label,
                disabled,
                checked,
                ..
            } => Self::Item {
                id,
                label,
                danger: true,
                disabled,
                checked,
            },
            other => other,
        }
    }
    pub fn disabled(self, value: bool) -> Self {
        match self {
            Self::Item {
                id,
                label,
                danger,
                checked,
                ..
            } => Self::Item {
                id,
                label,
                danger,
                checked,
                disabled: value,
            },
            Self::Submenu { label, items, .. } => Self::Submenu {
                label,
                items,
                disabled: value,
            },
            other => other,
        }
    }
    pub fn checked(self, value: bool) -> Self {
        match self {
            Self::Item {
                id,
                label,
                danger,
                disabled,
                ..
            } => Self::Item {
                id,
                label,
                danger,
                disabled,
                checked: value,
            },
            other => other,
        }
    }
    fn enabled(&self) -> bool {
        matches!(
            self,
            Self::Item {
                disabled: false,
                ..
            } | Self::Submenu {
                disabled: false,
                ..
            }
        )
    }
}
pub enum ContextMenuEvent {
    Select(String),
    Close,
}
pub struct UiContextMenu {
    entries: Vec<MenuEntry>,
    anchor: Point<Pixels>,
    open: bool,
    path: Vec<usize>,
    selected: Vec<usize>,
    focus: FocusHandle,
    opener: Option<FocusHandle>,
    handles: HashMap<Vec<usize>, FocusHandle>,
    scrolls: HashMap<Vec<usize>, ScrollHandle>,
    row_bounds: HashMap<Vec<usize>, Bounds<Pixels>>,
    panel_bounds: HashMap<Vec<usize>, Bounds<Pixels>>,
    generation: u64,
}
impl EventEmitter<ContextMenuEvent> for UiContextMenu {}
impl UiContextMenu {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            entries: vec![],
            anchor: point(px(0.), px(0.)),
            open: false,
            path: vec![],
            selected: vec![],
            focus: cx.focus_handle(),
            opener: None,
            handles: HashMap::new(),
            scrolls: HashMap::new(),
            row_bounds: HashMap::new(),
            panel_bounds: HashMap::new(),
            generation: 0,
        }
    }
    pub fn is_open(&self) -> bool {
        self.open
    }
    pub fn open(
        &mut self,
        anchor: Point<Pixels>,
        entries: Vec<MenuEntry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Replacing an open context menu keeps its original focus destination.
        if !self.open {
            self.opener = window.focused(cx);
        }
        self.generation = self.generation.wrapping_add(1);
        self.anchor = anchor;
        self.entries = entries;
        self.path.clear();
        self.selected.clear();
        self.handles.clear();
        self.scrolls.clear();
        self.row_bounds.clear();
        self.panel_bounds.clear();
        self.open = true;
        self.focus.focus(window, cx);
        self.move_selection(1, false, window, cx);
        cx.notify();
    }
    /// Open below a trigger, aligning the panel's right edge with the trigger.
    pub fn open_below_right(
        &mut self,
        trigger: Bounds<Pixels>,
        entries: Vec<MenuEntry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let width = menu_width(window.viewport_size());
        self.open(
            point(trigger.right() - px(width), trigger.bottom() + px(4.)),
            entries,
            window,
            cx,
        );
    }

    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        self.generation = self.generation.wrapping_add(1);
        self.path.clear();
        self.selected.clear();
        self.entries.clear();
        self.handles.clear();
        self.scrolls.clear();
        self.row_bounds.clear();
        self.panel_bounds.clear();
        if let Some(opener) = self.opener.take() {
            opener.focus(window, cx);
        } else {
            window.blur(cx);
        }
        cx.emit(ContextMenuEvent::Close);
        cx.notify();
    }
    fn entries_at(&self, path: &[usize]) -> &[MenuEntry] {
        let mut entries = self.entries.as_slice();
        for index in path {
            let Some(MenuEntry::Submenu { items, .. }) = entries.get(*index) else {
                return &[];
            };
            entries = items;
        }
        entries
    }
    fn entry_at(&self, path: &[usize]) -> Option<&MenuEntry> {
        let (index, prefix) = path.split_last()?;
        self.entries_at(prefix).get(*index)
    }
    fn focus_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((index, panel)) = self.selected.split_last() {
            if let Some(scroll) = self.scrolls.get(panel) {
                scroll.scroll_to_item(*index);
            }
        }
        let focus = self
            .handles
            .entry(self.selected.clone())
            .or_insert_with(|| cx.focus_handle())
            .clone();
        focus.focus(window, cx);
    }
    fn move_selection(
        &mut self,
        direction: i32,
        end: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panel = navigation_panel(&self.selected, &self.path);
        let current = self
            .selected
            .strip_prefix(panel.as_slice())
            .and_then(|rest| rest.first())
            .copied();
        let Some(next) = next_enabled(self.entries_at(&panel), current, direction, end) else {
            return;
        };
        self.selected = panel;
        self.selected.push(next);
        self.focus_selected(window, cx);
        cx.notify();
    }
    fn close_submenu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.path.is_empty() {
            return;
        }
        self.selected = self.path.clone();
        self.path.pop();
        self.focus_selected(window, cx);
        cx.notify();
    }
    fn activate(
        &mut self,
        path: Vec<usize>,
        keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.entry_at(&path).cloned() {
            Some(MenuEntry::Item {
                id,
                disabled: false,
                ..
            }) => {
                self.close(window, cx);
                cx.emit(ContextMenuEvent::Select(id));
            }
            Some(MenuEntry::Submenu {
                disabled: false, ..
            }) => {
                if !keyboard && self.path.starts_with(&path) {
                    self.selected = path.clone();
                    self.path = path[..path.len() - 1].to_vec();
                    self.focus_selected(window, cx);
                } else {
                    self.path = path.clone();
                    self.selected = path;
                    if keyboard {
                        // ArrowRight moves into the child panel. Hover/Enter
                        // leave focus on its trigger, where Up/Down continue
                        // navigating the trigger's sibling items.
                        self.selected.clear();
                        self.move_selection(1, false, window, cx);
                    }
                }
                cx.notify();
            }
            _ => {}
        }
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        let intent = key_intent(
            &event.keystroke.key,
            self.entry_at(&self.selected),
            !self.path.is_empty(),
        );
        match intent {
            KeyIntent::Move(direction, end) => self.move_selection(direction, end, window, cx),
            KeyIntent::OpenSubmenu => self.activate(self.selected.clone(), true, window, cx),
            KeyIntent::Activate => self.activate(self.selected.clone(), false, window, cx),
            KeyIntent::CloseSubmenu => self.close_submenu(window, cx),
            KeyIntent::Close => self.close(window, cx),
            KeyIntent::Ignore => return,
        }
        window.prevent_default();
        cx.stop_propagation();
    }
    /// Defer geometry publication until GPUI releases layout borrows. Unchanged
    /// bounds do not notify, so measured placement settles after one correction.
    fn marker(&self, path: Vec<usize>, panel: bool, cx: &Context<Self>) -> impl IntoElement {
        let entity = cx.entity().downgrade();
        let generation = self.generation;
        canvas(
            move |bounds, _, cx| {
                // An absolute inset-zero child spans the padding box. Include
                // the panel's one-pixel outline when placing its border box.
                let bounds = if panel {
                    Bounds {
                        origin: bounds.origin - point(px(1.), px(1.)),
                        size: bounds.size + size(px(2.), px(2.)),
                    }
                } else {
                    bounds
                };
                cx.defer(move |cx| {
                    entity
                        .update(cx, |this, cx| {
                            if !this.open || this.generation != generation {
                                return;
                            }
                            let geometry = if panel {
                                &mut this.panel_bounds
                            } else {
                                &mut this.row_bounds
                            };
                            if geometry.get(&path) != Some(&bounds) {
                                geometry.insert(path, bounds);
                                cx.notify();
                            }
                        })
                        .ok();
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
    }
    fn panel(
        &mut self,
        path: Vec<usize>,
        origin: Point<Pixels>,
        width: f32,
        theme: Theme,
        viewport: Size<Pixels>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entries = self.entries_at(&path).to_vec();
        let scroll = self.scrolls.entry(path.clone()).or_default().clone();
        let mut panel = div()
            .id(ElementId::Name(format!("context-panel-{path:?}").into()))
            .absolute()
            .left(origin.x)
            .top(origin.y)
            .w(px(width))
            .max_h(px((f32::from(viewport.height) - 16.).max(0.)))
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .flex()
            .flex_col()
            .p(px(theme.space1))
            .rounded(px(theme.radius_sm))
            .border_1()
            .border_color(theme.outline)
            .bg(theme.surface_raised)
            .shadow(vec![BoxShadow {
                color: theme.shadow_md.color.into(),
                offset: point(px(0.), px(theme.shadow_md.dy)),
                blur_radius: px(theme.shadow_md.blur),
                spread_radius: px(0.),
                inset: false,
            }])
            .text_color(theme.text)
            .role(Role::Menu)
            .occlude();
        for (index, entry) in entries.into_iter().enumerate() {
            if matches!(entry, MenuEntry::Separator) {
                panel = panel.child(
                    div()
                        .w_full()
                        .h(px(1.))
                        .flex_shrink_0()
                        .my(px(theme.space1))
                        .bg(theme.outline),
                );
                continue;
            }
            let mut target = path.clone();
            target.push(index);
            let focus = self
                .handles
                .entry(target.clone())
                .or_insert_with(|| cx.focus_handle())
                .clone();
            let disabled = !entry.enabled();
            let (label, danger, checked, submenu) = match entry {
                MenuEntry::Item {
                    label,
                    danger,
                    checked,
                    ..
                } => (label, danger, checked, false),
                MenuEntry::Submenu { label, .. } => (label, false, false, true),
                _ => unreachable!(),
            };
            let mut row = div()
                .id(ElementId::Name(format!("context-item-{target:?}").into()))
                .relative()
                .track_focus(&focus)
                .tab_index(if disabled { -1 } else { 0 })
                .flex()
                .flex_shrink_0()
                .items_center()
                .justify_between()
                .gap(px(theme.space3))
                .w_full()
                .min_h(px(40.))
                .py(px(theme.space2))
                .px(px(theme.space3))
                .rounded(px(theme.radius_sm))
                .text_size(px(theme.font_size_md))
                .line_height(relative(theme.line_height_normal))
                .text_color(if danger { theme.danger } else { theme.text })
                .role(Role::MenuItem)
                .child(div().flex_1().min_w_0().child(label));
            if checked {
                row = row.child(Icon::Check.render(theme.text, 16.));
            } else if submenu {
                row = row
                    .aria_expanded(self.path.starts_with(&target))
                    .child(Icon::ChevronRight.render(theme.text_muted, 16.));
            }
            if disabled {
                row = row.opacity(0.45).cursor_not_allowed();
            } else {
                let hover_target = target.clone();
                let click_target = target.clone();
                let ring = vec![BoxShadow {
                    color: theme.focus.into(),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(2.),
                    inset: true,
                }];
                row = row
                    .cursor_pointer()
                    .hover(move |style| style.bg(theme.surface_selected))
                    .focus_visible(move |style| {
                        style.bg(theme.surface_selected).shadow(ring.clone())
                    })
                    .on_hover(cx.listener(move |this, entered, window, cx| {
                        if *entered && this.open {
                            let next_path = if submenu {
                                hover_target.clone()
                            } else {
                                hover_target[..hover_target.len() - 1].to_vec()
                            };
                            if this.selected != hover_target || this.path != next_path {
                                this.selected = hover_target.clone();
                                this.path = next_path;
                                this.focus_selected(window, cx);
                                cx.notify();
                            }
                        }
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.activate(click_target.clone(), false, window, cx)
                    }));
            }
            row = row.child(self.marker(target, false, cx));
            panel = panel.child(row);
        }
        panel.child(self.marker(path, true, cx)).into_any_element()
    }
}
fn estimated_height(entries: &[MenuEntry]) -> f32 {
    10. + entries
        .iter()
        .map(|entry| {
            if matches!(entry, MenuEntry::Separator) {
                9.
            } else {
                40.
            }
        })
        .sum::<f32>()
}
fn clamp_axis(value: f32, size: f32, viewport: f32) -> f32 {
    let minimum = 8_f32.min((viewport - size).max(0.));
    let maximum = minimum.max(viewport - size - 8.);
    value.clamp(minimum, maximum)
}
fn menu_width(viewport: Size<Pixels>) -> f32 {
    220_f32.min((f32::from(viewport.width) - 16.).max(1.))
}

fn axis(anchor: f32, size: f32, viewport: f32) -> f32 {
    let placed = if anchor + size <= viewport - 8. {
        anchor
    } else if anchor - size >= 8. {
        anchor - size
    } else {
        anchor
    };
    clamp_axis(placed, size, viewport)
}
fn submenu_origin(
    anchor: Bounds<Pixels>,
    size: Size<Pixels>,
    viewport: Size<Pixels>,
) -> Point<Pixels> {
    let width = f32::from(size.width);
    let viewport_width = f32::from(viewport.width);
    let right = f32::from(anchor.right()) + 4.;
    let left = f32::from(anchor.left()) - width - 4.;
    let x = if right + width <= viewport_width - 8. {
        right
    } else if left >= 8. {
        left
    } else {
        clamp_axis(right, width, viewport_width)
    };
    point(
        px(clamp_axis(x, width, viewport_width)),
        px(clamp_axis(
            f32::from(anchor.top()) - 6.,
            f32::from(size.height),
            f32::from(viewport.height),
        )),
    )
}
impl Render for UiContextMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div().id("context-menu-root").absolute().inset_0();
        if !self.open {
            return root;
        }
        let mut root = root
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(Self::key));
        let theme = Theme::current(cx);
        let viewport = window.viewport_size();
        let width = menu_width(viewport);
        root = root.child(
            div()
                .id("context-outside")
                .absolute()
                .inset_0()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.close(window, cx)),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, _, window, cx| this.close(window, cx)),
                ),
        );
        let height = self
            .panel_bounds
            .get(&vec![])
            .map(|bounds| f32::from(bounds.size.height))
            .unwrap_or_else(|| estimated_height(&self.entries))
            .min((f32::from(viewport.height) - 16.).max(0.));
        let mut origin = point(
            px(axis(
                f32::from(self.anchor.x),
                width,
                f32::from(viewport.width),
            )),
            px(axis(
                f32::from(self.anchor.y),
                height,
                f32::from(viewport.height),
            )),
        );
        root = root.child(self.panel(vec![], origin, width, theme, viewport, cx));
        let path = self.path.clone();
        let mut prefix = vec![];
        for index in path {
            let entries = self.entries_at(&prefix);
            let Some(MenuEntry::Submenu { items, .. }) = entries.get(index) else {
                break;
            };
            let fallback = Bounds {
                origin: point(
                    origin.x + px(5.),
                    origin.y
                        + px(5.
                            + entries[..index]
                                .iter()
                                .map(|entry| {
                                    if matches!(entry, MenuEntry::Separator) {
                                        9.
                                    } else {
                                        40.
                                    }
                                })
                                .sum::<f32>()),
                ),
                size: size(px(width - 10.), px(40.)),
            };
            prefix.push(index);
            let anchor = self.row_bounds.get(&prefix).copied().unwrap_or(fallback);
            let measured = self
                .panel_bounds
                .get(&prefix)
                .map(|bounds| bounds.size)
                .unwrap_or_else(|| {
                    size(
                        px(width),
                        px(estimated_height(items).min((f32::from(viewport.height) - 16.).max(0.))),
                    )
                });
            origin = submenu_origin(anchor, measured, viewport);
            root = root.child(self.panel(prefix.clone(), origin, width, theme, viewport, cx));
        }
        root
    }
}
#[cfg(test)]
mod tests {
    use super::{axis, submenu_origin};
    use gpui_kit::{point, px, size, Bounds};
    #[test]
    fn menu_flips_and_stays_inside_viewport() {
        assert_eq!(axis(900., 220., 1000.), 680.);
        assert_eq!(axis(3., 220., 1000.), 8.);
        assert_eq!(axis(300., 220., 400.), 80.);
        assert!(axis(30., 180., 200.) <= 12.);
    }
    #[test]
    fn submenu_uses_measured_row_bounds_and_source_small_viewport_fallback() {
        let row = Bounds {
            origin: point(px(172.), px(310.)),
            size: size(px(220.), px(65.)),
        };
        let placed = submenu_origin(row, size(px(220.), px(170.)), size(px(400.), px(450.)));
        assert_eq!(placed, point(px(172.), px(272.)));
        let scrolled = Bounds {
            origin: point(px(172.), px(50.)),
            ..row
        };
        assert_eq!(
            submenu_origin(
                scrolled,
                size(px(220.), px(170.)),
                size(px(1000.), px(450.))
            ),
            point(px(396.), px(44.))
        );
    }
}
