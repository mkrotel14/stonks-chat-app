//! Offline shared-control geometry fixture. All callbacks modify memory only.
//! Run with --light-focus and/or --focus-switch; --wide gives horizontal
//! permission rows. Resize below 1100px to compare stacked permission rows.
use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::component::theme::{Theme as KitTheme, ThemeMode};
use gpui_kit::*;
use stonks_ui::{
    components::select::{IndexPath, SearchableVec},
    components::{
        input_field, settings_field, settings_row, BotBadge, Button, ButtonVariant, ColorSwatch,
        ContextMenuEvent, MenuEntry, PermissionEditor, SettingsInput, Switch, UiAccordion,
        UiCloseButton, UiColorPicker, UiColorPickerEvent, UiColorPickerState, UiContextMenu,
        UiSelect, UiSelectOption, UiSelectState,
    },
    fonts,
    theme::CurrentTheme,
    Theme,
};

struct Fields {
    focus: FocusHandle,
    switch_focus: FocusHandle,
    name: Entity<InputState>,
    disabled: Entity<InputState>,
    description: Entity<TextareaState>,
    select: Entity<UiSelectState>,
    picker: Entity<UiColorPickerState>,
    disabled_picker: Entity<UiColorPickerState>,
    enabled_on: bool,
    enabled_off: bool,
    permissions_open: bool,
    disabled_permissions_open: bool,
    allow: u64,
    deny: u64,
    preset: usize,
    status: String,
}
struct Preview {
    fields: Vec<Fields>,
    menu: Entity<UiContextMenu>,
    active_panel: usize,
    menu_panel: usize,
    _subscriptions: Vec<Subscription>,
}
impl Preview {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let fields = (0..2)
            .map(|_| {
                let name = cx.new(|cx| InputState::new(window, cx));
                name.update(cx, |input, cx| {
                    input.set_value("Native settings fixture", window, cx)
                });
                let description = cx.new(|cx| TextareaState::new(window, cx));
                description.update(cx, |input, cx| {
                    input.set_value(
                        "Description with multiple lines.\nSemantic tokens in both themes.",
                        window,
                        cx,
                    )
                });
                let select = cx.new(|cx| {
                    UiSelectState::new(
                        SearchableVec::new(vec![
                            UiSelectOption::new("", "Disabled destination"),
                            UiSelectOption::new("general", "# General"),
                            UiSelectOption::new("restricted", "Unavailable channel").disabled(true),
                        ]),
                        Some(IndexPath::new(0)),
                        window,
                        cx,
                    )
                });
                let disabled = cx.new(|cx| InputState::new(window, cx));
                disabled.update(cx, |input, cx| {
                    input.set_value("Disabled field", window, cx)
                });
                let picker = cx.new(|cx| UiColorPickerState::new(window, cx));
                let disabled_picker = cx.new(|cx| UiColorPickerState::new(window, cx));
                for picker in [&picker, &disabled_picker] {
                    picker.update(cx, |picker, cx| picker.set_value(rgb(0x5865f2), window, cx));
                }
                Fields {
                    focus: cx.focus_handle(),
                    switch_focus: cx.focus_handle(),
                    name,
                    disabled,
                    description,
                    select,
                    picker,
                    disabled_picker,
                    enabled_on: true,
                    enabled_off: false,
                    permissions_open: false,
                    disabled_permissions_open: false,
                    allow: (1 << 0) | (1 << 41),
                    deny: (1 << 7) | (1 << 38),
                    preset: 0,
                    status: "Ready · local preview".into(),
                }
            })
            .collect::<Vec<_>>();
        let menu = cx.new(UiContextMenu::new);
        let mut subscriptions = vec![
            cx.observe(&menu, |_, _, cx| cx.notify()),
            cx.subscribe(&menu, |this, _, event, cx| {
                if let ContextMenuEvent::Select(id) = event {
                    this.fields[this.menu_panel].status =
                        format!("Menu action: {id} · no external effect");
                    cx.notify();
                }
            }),
        ];
        for (index, field) in fields.iter().enumerate() {
            subscriptions.push(cx.observe(&field.name, |_, _, cx| cx.notify()));
            subscriptions.push(cx.observe(&field.description, |_, _, cx| cx.notify()));
            subscriptions.push(cx.observe(&field.picker, |_, _, cx| cx.notify()));
            subscriptions.push(
                cx.on_focus_in(&field.focus, window, move |this, window, cx| {
                    this.activate_theme(index, window, cx);
                }),
            );
            subscriptions.push(cx.subscribe(&field.picker, move |this, _, event, cx| {
                if let UiColorPickerEvent::Change(Some(color)) = event {
                    let color = Rgba::from(*color);
                    this.fields[index].status = format!(
                        "Colour: #{:02x}{:02x}{:02x}",
                        (color.r * 255.).round() as u8,
                        (color.g * 255.).round() as u8,
                        (color.b * 255.).round() as u8
                    );
                    cx.notify();
                }
            }));
        }
        let focused_panel = usize::from(std::env::args().any(|arg| arg == "--light-focus"));
        if std::env::args().any(|arg| arg == "--focus-switch") {
            fields[focused_panel].switch_focus.focus(window, cx);
        } else {
            fields[focused_panel]
                .name
                .update(cx, |input, cx| input.focus(window, cx));
        }
        install_theme(focused_panel, window, cx);
        Self {
            fields,
            menu,
            active_panel: focused_panel,
            menu_panel: focused_panel,
            _subscriptions: subscriptions,
        }
    }
    fn activate_theme(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_panel != index {
            self.active_panel = index;
            install_theme(index, window, cx);
        }
    }
    fn open_menu(
        &mut self,
        index: usize,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.activate_theme(index, window, cx);
        self.menu_panel = index;
        let scrolling = (1..=24)
            .map(|number| {
                MenuEntry::action(
                    format!("scroll-item-{number}"),
                    format!("Scrollable result {number}"),
                )
            })
            .collect();
        let entries = vec![
            MenuEntry::action("inspect", "Inspect local fixture"),
            MenuEntry::action("disabled", "Disabled action").disabled(true),
            MenuEntry::Submenu { label: "Notifications".into(), disabled: false, items: vec![
                MenuEntry::action("default", "Default").checked(true),
                MenuEntry::action("all", "All messages"),
                MenuEntry::Submenu { label: "Nested options".into(), disabled: false, items: vec![
                    MenuEntry::action("nested", "Nested enabled action"),
                    MenuEntry::action("nested-disabled", "Nested disabled action").disabled(true),
                ]},
                MenuEntry::action("long", "A deliberately long translated menu label that wraps to verify measured submenu placement"),
            ]},
            MenuEntry::Submenu { label: "Scrollable submenu (24 rows)".into(), disabled: false, items: scrolling },
            MenuEntry::Separator,
            MenuEntry::action("danger-preview", "Danger styling preview").danger(),
        ];
        self.menu
            .update(cx, |menu, cx| menu.open(position, entries, window, cx));
    }
    fn panel(
        &self,
        index: usize,
        theme: Theme,
        content_width: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let fields = &self.fields[index];
        let entity = cx.entity().downgrade();
        let editor = PermissionEditor::new(
            format!("fixture-permissions-{index}"),
            theme,
            fields.allow,
            fields.deny,
            move |allow, deny, _, cx| {
                entity
                    .update(cx, |this, cx| {
                        this.fields[index].allow = allow;
                        this.fields[index].deny = deny;
                        this.fields[index].status =
                            format!("Permission masks: allow {allow:#x}, deny {deny:#x}");
                        cx.notify();
                    })
                    .ok();
            },
        )
        .compact(content_width - 34. <= 480.);
        let mut presets = div().flex().items_center().gap(px(theme.space2));
        for (preset, color) in [0x5865f2, 0x2a9d8f, 0xe76f51].into_iter().enumerate() {
            presets = presets.child(
                ColorSwatch::new(
                    ElementId::Name(format!("preset-{index}-{preset}").into()),
                    theme,
                    rgb(color),
                )
                .selected(fields.preset == preset)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.fields[index].preset = preset;
                    this.fields[index]
                        .picker
                        .update(cx, |picker, cx| picker.set_value(rgb(color), window, cx));
                    cx.notify();
                })),
            );
        }
        div().id(ElementId::Name(format!("settings-theme-{index}").into()))
            .track_focus(&fields.focus).flex_1().min_w_0().h_full().overflow_y_scroll().flex().flex_col()
            .p(px(36.)).gap(px(theme.space4)).bg(theme.surface).text_color(theme.text)
            .line_height(relative(theme.line_height_normal))
            .child(div().flex().items_start().justify_between().flex_shrink_0()
                .child(div().text_size(px(27.)).font_weight(FontWeight::BOLD).child(if index==0 {"Dark controls"} else {"Light controls"}))
                .child(UiCloseButton::new("close", theme).on_click(cx.listener(move |this, _, _, cx| {
                    this.fields[index].status="Close button activated · fixture stays open".into();cx.notify();
                }))))
            .child(div().text_size(px(theme.font_size_sm)).text_color(theme.text_muted)
                .child("Offline: Tab/Shift-Tab, Enter/Space and pointer actions change this preview only."))
            .child(div().flex().items_center().flex_wrap().gap(px(theme.space2)).flex_shrink_0()
                .child(Button::new("open-menu", theme).variant(ButtonVariant::Secondary).label("Open context menu")
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| this.open_menu(index, event.position(), window, cx))))
                .child(BotBadge::new(theme)))
            .child(div().flex().flex_col().gap(px(theme.space2)).flex_shrink_0()
                .child(settings_row(&theme, "Enabled switch — starts on", "Use Tab, Enter or Space to toggle.",
                    Switch::new("enabled-on", theme).label("Enabled switch, starts on").track_focus(&fields.switch_focus).checked(fields.enabled_on)
                        .on_change(cx.listener(move |this, value, _, cx| {this.fields[index].enabled_on=*value;this.fields[index].status=format!("First switch: {value}");cx.notify();}))))
                .child(settings_row(&theme, "Enabled switch — starts off", "Compare track and thumb contrast.",
                    Switch::new("enabled-off", theme).label("Enabled switch, starts off").checked(fields.enabled_off)
                        .on_change(cx.listener(move |this, value, _, cx| {this.fields[index].enabled_off=*value;this.fields[index].status=format!("Second switch: {value}");cx.notify();}))))
                .child(div().flex().items_center().justify_between().gap(px(theme.space3)).py(px(theme.space2))
                    .child("Disabled switches")
                    .child(div().flex().gap(px(theme.space3))
                        .child(Switch::new("disabled-on", theme).label("Disabled checked switch").checked(true).disabled(true))
                        .child(Switch::new("disabled-off", theme).label("Disabled unchecked switch").disabled(true)))))
            .child(settings_field(&theme, "Role colours — presets and native picker", "").flex_shrink_0()
                .child(div().flex().items_center().flex_wrap().gap(px(theme.space3))
                    .child(presets).child(UiColorPicker::new(&fields.picker, theme).label("Enabled role colour picker"))
                    .child(ColorSwatch::new("disabled-swatch", theme, rgb(0xe76f51)).selected(true).disabled(true))
                    .child(UiColorPicker::new(&fields.disabled_picker, theme).label("Disabled role colour picker").disabled(true))))
            .child(div().text_size(px(theme.font_size_xs)).text_color(theme.text_muted).child(fields.status.clone()))
            .child(input_field(&theme, "Server name", &format!("{}/32", fields.name.read(cx).value().encode_utf16().count()),
                SettingsInput::new(&fields.name, theme).label("Server name")).flex_shrink_0())
            .child(settings_field(&theme, "Description", "").flex_shrink_0().child(SettingsInput::textarea(&fields.description, theme).label("Description")))
            .child(settings_row(&theme, "User joined", "Choose where this event is posted.",
                div().w(px(190.)).child(UiSelect::new(&fields.select, theme).min_height(42.).label("User joined"))))
            .child(settings_field(&theme, "Disabled name", "").flex_shrink_0().child(SettingsInput::new(&fields.disabled, theme).label("Disabled name").disabled(true)))
            .child(div().flex().flex_wrap().gap(px(theme.space2)).flex_shrink_0()
                .child(Button::new("save", theme).label("Local save preview").on_click(cx.listener(move |this, _, _, cx| {this.fields[index].status="Local save preview activated".into();cx.notify();})))
                .child(Button::new("disabled-save", theme).label("Disabled save").disabled(true)))
            .child(UiAccordion::new("permissions", theme, "Interactive permission overrides").subtitle("Allow / inherit / deny, including bits 38 and 41")
                .open(fields.permissions_open).on_toggle(cx.listener(move |this, _, _, cx| {this.fields[index].permissions_open=!this.fields[index].permissions_open;cx.notify();}))
                .child(editor))
            .child(UiAccordion::new("disabled-permissions", theme, "Disabled permission editor").subtitle("Expand to inspect disabled colours and keyboard exclusion")
                .open(fields.disabled_permissions_open).on_toggle(cx.listener(move |this, _, _, cx| {this.fields[index].disabled_permissions_open=!this.fields[index].disabled_permissions_open;cx.notify();}))
                .child(PermissionEditor::new(format!("fixture-disabled-permissions-{index}"),theme,fields.allow,fields.deny,|_,_,_,_|{})
                    .compact(content_width-34. <= 480.).disabled(true)))
    }
}
fn install_theme(index: usize, window: &mut Window, cx: &mut App) {
    let theme = if index == 0 {
        Theme::dark()
    } else {
        Theme::light()
    };
    cx.set_global(CurrentTheme(theme));
    KitTheme::change(
        if index == 0 {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        },
        Some(window),
        cx,
    );
}
impl Render for Preview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content_width = f32::from(window.viewport_size().width) / 2. - 72.;
        div()
            .relative()
            .size_full()
            .flex()
            .font_family(fonts::FAMILY)
            .child(self.panel(0, Theme::dark(), content_width, cx))
            .child(self.panel(1, Theme::light(), content_width, cx))
            .child(self.menu.clone())
    }
}
fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            fonts::register(cx);
            let wide = std::env::args().any(|arg| arg == "--wide");
            gpui_kit::open_window(
                WindowOptions {
                    app_id: Some("stonks-settings-preview".into()),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Offline settings controls — dark / light".into()),
                        ..Default::default()
                    }),
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        if wide {
                            size(px(1500.), px(900.))
                        } else {
                            size(px(1100.), px(760.))
                        },
                        cx,
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| Preview::new(window, cx)),
            )
            .expect("settings preview");
            cx.activate(true);
        });
}
