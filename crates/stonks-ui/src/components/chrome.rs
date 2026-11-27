//! Window chrome helpers for client-side decorated windows: drag regions
//! and min/max/close controls, following the Zed/gpui-kit titlebar pattern.
//! Shared by every surface that doubles as a titlebar (the auth header,
//! the shell pane header, and future pop-out windows).

use gpui_kit::base::InteractiveElementExt;
use gpui_kit::*;

use crate::icons::Icon;
use crate::theme::{mix_color, Theme};

struct ChromeDrag {
    pending: bool,
}

/// Drag-to-move, double-click to zoom, right-click for the compositor's
/// window menu. Attach only to non-interactive regions, so pressing a
/// button can never start a drag.
pub fn draggable_chrome(
    element: Stateful<Div>,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let state = window.use_state(cx, |_, _| ChromeDrag { pending: false });
    element
        .h_full()
        .on_mouse_down_out(window.listener_for(&state, |state, _, _, _| {
            state.pending = false;
        }))
        .on_mouse_down(
            MouseButton::Left,
            window.listener_for(&state, |state, _, _, _| state.pending = true),
        )
        .on_mouse_up(
            MouseButton::Left,
            window.listener_for(&state, |state, _, _, _| state.pending = false),
        )
        .on_mouse_move(window.listener_for(&state, |state, _, window, _| {
            if state.pending {
                state.pending = false;
                window.start_window_move();
            }
        }))
        .on_double_click(|_, window, _| window.zoom_window())
        .on_mouse_down(MouseButton::Right, |event, window, _| {
            window.show_window_menu(event.position);
        })
}

/// Min/max/close honoring the compositor's reported capabilities. Tiling
/// compositors (Hyprland, Sway, …) manage their own windows and report no
/// minimize support, so the whole group is hidden there; stacking WMs and
/// Windows get the controls. Gating on the negotiated decoration mode is
/// not an option: Hyprland answers xdg-decoration with server-side mode
/// yet draws nothing itself.
pub fn window_controls(theme: &Theme, window: &Window) -> Div {
    window_controls_with_close(theme, window, |_, window, _| window.remove_window())
}

pub fn window_controls_with_close(
    theme: &Theme,
    window: &Window,
    close: fn(&ClickEvent, &mut Window, &mut App),
) -> Div {
    let supported = window.window_controls();
    let maximized = window.is_maximized();

    let is_windows = cfg!(target_os = "windows");
    let show_controls = is_windows || supported.minimize;

    let control = |id: &'static str,
                   icon: Icon,
                   color: Rgba,
                   hover_bg: Rgba,
                   on_click: fn(&ClickEvent, &mut Window, &mut App)| {
        div()
            .id(id)
            .flex()
            .w(px(32.))
            .h(px(32.))
            .items_center()
            .justify_center()
            .rounded(px(theme.radius_sm))
            .cursor_pointer()
            .hover(move |style| style.bg(hover_bg))
            .on_click(on_click)
            .child(icon.render(color, 16.))
    };

    let mut controls = div().flex().items_center().gap(px(theme.space2));
    if show_controls && (is_windows || supported.minimize) {
        controls = controls.child(control(
            "chrome-minimize",
            Icon::Minus,
            theme.text_muted,
            theme.surface_interactive,
            |_, window, _| window.minimize_window(),
        ));
    }
    if show_controls && (is_windows || supported.maximize) {
        controls = controls.child(control(
            "chrome-maximize",
            if maximized { Icon::Copy } else { Icon::Square },
            theme.text_muted,
            theme.surface_interactive,
            |_, window, _| window.zoom_window(),
        ));
    }
    if show_controls {
        controls = controls.child(control(
            "chrome-close",
            Icon::X,
            theme.danger,
            mix_color(theme.surface_raised, theme.danger, 0.14),
            close,
        ));
    }
    controls
}
