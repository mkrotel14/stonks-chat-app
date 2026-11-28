//! Native process, window and launcher lifecycle. Product routing belongs to the session UI.
mod deep_links;
pub(crate) mod explorer_layout;
#[cfg_attr(target_os = "windows", path = "instance_windows.rs")]
mod instance;
#[cfg_attr(target_os = "windows", path = "startup_windows.rs")]
mod startup;
#[cfg(target_os = "linux")]
mod tray;
#[cfg(target_os = "windows")]
pub(crate) mod tray_windows;
mod window_state;

pub(crate) use deep_links::DeepLink;
pub(crate) use startup::{set_start_with_computer, starts_with_computer};

use crate::features::app::AppView;
use gpui_kit::{
    px, size, AnyWindowHandle, App, AppContext, Global, QuitMode, Styled, TitlebarOptions, Window,
    WindowDecorations, WindowOptions,
};
use std::{
    cell::{Cell, RefCell},
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

fn update_blocks_close(cx: &App) -> bool {
    crate::features::app::updater::Updater::shared(cx).is_some_and(|updater| {
        use crate::features::app::updater::Status;
        matches!(
            updater.read(cx).status,
            Status::Downloading | Status::Verifying | Status::Installing
        )
    })
}

#[cfg(any(target_os = "linux", test))]
fn is_tiling_desktop(desktop: &str) -> bool {
    desktop.split(':').any(|name| {
        ["Hyprland", "Omarchy", "sway", "niri", "i3"]
            .iter()
            .any(|tiler| name.trim().eq_ignore_ascii_case(tiler))
    })
}

fn prefers_client_chrome() -> bool {
    #[cfg(target_os = "linux")]
    {
        use std::sync::OnceLock;
        static PREFERENCE: OnceLock<bool> = OnceLock::new();
        *PREFERENCE.get_or_init(|| {
            [
                "XDG_CURRENT_DESKTOP",
                "XDG_SESSION_DESKTOP",
                "DESKTOP_SESSION",
            ]
            .iter()
            .filter_map(|key| std::env::var(key).ok())
            .any(|desktop| is_tiling_desktop(&desktop))
                || std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some_and(|v| !v.is_empty())
        })
    }
    #[cfg(not(target_os = "linux"))]
    false
}

fn uses_main_client_chrome(window: &Window) -> bool {
    // Wayland compositors without server-side decorations (e.g. GNOME)
    // need the same client chrome fallback as sessions that request it.
    cfg!(target_os = "linux")
        && (prefers_client_chrome()
            || matches!(
                window.window_decorations(),
                gpui_kit::Decorations::Client { .. }
            ))
}

pub(crate) fn main_window_drag_region(
    element: gpui_kit::Stateful<gpui_kit::Div>,
    window: &mut Window,
    cx: &mut App,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    if uses_main_client_chrome(window) {
        stonks_ui::components::draggable_chrome(element, window, cx)
    } else {
        element.h_full()
    }
}

pub(crate) fn main_window_controls(
    theme: &stonks_ui::Theme,
    window: &Window,
) -> Option<gpui_kit::Div> {
    uses_main_client_chrome(window).then(|| {
        stonks_ui::components::window_controls_with_close(theme, window, |_, window, cx| {
            if !update_blocks_close(cx) {
                window.remove_window();
            }
        })
    })
}

pub(crate) const APP_ID: &str = "com.rdxlabs.stonks.gpui";

/// Latest validated launch link, retained across startup and authentication.
/// Observe this global for warm launches; also check it when the session becomes ready.
pub(crate) struct PendingDeepLink(Option<DeepLink>);
impl Global for PendingDeepLink {}

impl PendingDeepLink {
    /// Take only when the active session can route the link. An empty read does not notify observers.
    pub(crate) fn take(cx: &mut App) -> Option<DeepLink> {
        cx.try_global::<Self>()?.0.as_ref()?;
        cx.global_mut::<Self>().0.take()
    }
}

pub(crate) struct Launch {
    instance: Option<instance::Instance>,
    link: Option<DeepLink>,
    persist: bool,
    pub(crate) automatic_renderer_override: bool,
}

pub(crate) fn supports_start_minimized(cx: &App) -> bool {
    cfg!(target_os = "windows") || matches!(cx.compositor_name(), "X11" | "Wayland")
}

fn should_start_minimized(saved: bool, supported: bool, has_link: bool) -> bool {
    saved && supported && !has_link
}

impl Launch {
    /// Returns None after a launcher command or successful secondary activation.
    pub(crate) fn from_environment() -> io::Result<Option<Self>> {
        let arguments: Vec<_> = env::args_os().skip(1).collect();
        if arguments.len() == 1 {
            match arguments[0].to_str() {
                Some("--enable-autostart") => return set_start_with_computer(true).map(|()| None),
                Some("--disable-autostart") => {
                    return set_start_with_computer(false).map(|()| None)
                }
                _ => {}
            }
        }
        let automatic_renderer_override = arguments.iter().any(|arg| arg == "--automatic-renderer");
        let candidate = deep_links::first_valid(&arguments);
        let persist = env::var("STONKS_ALLOW_MULTI_INSTANCE").as_deref() != Ok("1");
        let instance = if persist {
            let Some(instance) =
                instance::Instance::acquire(candidate.as_ref().map(|(raw, _)| *raw))?
            else {
                return Ok(None);
            };
            Some(instance)
        } else {
            None
        };
        Ok(Some(Self {
            instance,
            link: candidate.map(|(_, link)| link),
            persist,
            automatic_renderer_override,
        }))
    }

    pub(crate) fn run(self, cx: &mut App) -> bool {
        let state = window_state::WindowState::load(cx, self.persist);
        let start_minimized = should_start_minimized(
            cx.global::<crate::preferences::NativePreferences>()
                .saved
                .native_start_minimized,
            supports_start_minimized(cx),
            self.link.is_some(),
        );
        let explorer_layout = explorer_layout::ExplorerLayout::load(self.persist);
        cx.set_global(explorer_layout.clone());
        cx.set_global(PendingDeepLink(self.link));
        let lifecycle = Rc::new(Lifecycle {
            instance: self.instance,
            explorer_layout,
            minimize_on_start: Cell::new(start_minimized),
            state: RefCell::new(state),
            window: Cell::new(None),
            hidden: Cell::new(false),
            #[cfg(target_os = "linux")]
            tray: RefCell::new(None),
            #[cfg(target_os = "windows")]
            windows_tray: RefCell::new(None),
        });
        lifecycle.run(cx)
    }
}

struct Lifecycle {
    explorer_layout: explorer_layout::ExplorerLayout,
    minimize_on_start: Cell<bool>,
    instance: Option<instance::Instance>,
    state: RefCell<window_state::WindowState>,
    window: Cell<Option<AnyWindowHandle>>,
    hidden: Cell<bool>,
    #[cfg(target_os = "linux")]
    tray: RefCell<Option<tray::Tray>>,
    #[cfg(target_os = "windows")]
    windows_tray: RefCell<Option<tray_windows::Tray>>,
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn tray_enabled(cx: &App) -> bool {
    cx.global::<crate::preferences::NativePreferences>()
        .saved
        .minimize_to_tray
}

impl Lifecycle {
    #[cfg(target_os = "linux")]
    fn sync_tray(&self, cx: &App) {
        let mut tray = self.tray.borrow_mut();
        if tray_enabled(cx) {
            tray.get_or_insert_with(tray::Tray::start);
        } else {
            tray.take();
        }
    }

    fn tray_available(&self, _cx: &App) -> bool {
        #[cfg(target_os = "linux")]
        {
            tray_enabled(_cx)
                && self
                    .tray
                    .borrow()
                    .as_ref()
                    .is_some_and(tray::Tray::available)
        }
        #[cfg(target_os = "windows")]
        {
            self.windows_tray
                .borrow()
                .as_ref()
                .is_some_and(tray_windows::Tray::available)
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            false
        }
    }

    fn flush(&self) {
        self.state.borrow_mut().flush();
        self.explorer_layout.flush();
    }

    fn open(self: &Rc<Self>, cx: &mut App) -> bool {
        let minimized = self.minimize_on_start.get();
        let bounds = self.state.borrow().bounds(cx);
        let options = WindowOptions {
            window_decorations: Some(if prefers_client_chrome() {
                WindowDecorations::Client
            } else {
                WindowDecorations::Server
            }),
            titlebar: Some(TitlebarOptions {
                title: Some("Stonks Chat".into()),
                ..Default::default()
            }),
            app_id: Some(APP_ID.into()),
            window_bounds: Some(bounds),
            window_min_size: Some(size(
                px(window_state::MIN_WIDTH),
                px(window_state::MIN_HEIGHT),
            )),
            focus: !minimized,
            ..Default::default()
        };
        // A recreated window needs a fresh view tree: AppView and its children
        // bind subscriptions, focus and window tasks to their original handle.
        // Authentication and geometry restore from disk, not transient UI state.
        match gpui_kit::open_window(options, cx, |window, cx| {
            self.attach_window(window, cx);
            cx.new(|cx| AppView::new(window, cx))
        }) {
            Ok((window, _)) => {
                self.window.set(Some(window));
                self.hidden.set(false);
                cx.set_quit_mode(QuitMode::LastWindowClosed);
                if !minimized {
                    cx.activate(true);
                }
                true
            }
            Err(error) => {
                // Do not leave a windowless primary consuming launcher activations.
                eprintln!("Could not open the Stonks window: {error}");
                // Linux starts its event loop after the launch callback returns.
                cx.spawn(async move |cx| {
                    cx.update(|cx| cx.quit());
                })
                .detach();
                false
            }
        }
    }

    fn attach_window(self: &Rc<Self>, window: &Window, cx: &App) {
        if self.minimize_on_start.get() {
            let lifecycle = Rc::downgrade(self);
            window.on_next_frame(move |window, _| {
                if lifecycle
                    .upgrade()
                    .is_some_and(|lifecycle| lifecycle.minimize_on_start.replace(false))
                {
                    window.minimize_window();
                }
            });
        }
        let lifecycle = Rc::downgrade(self);
        window.on_window_should_close(cx, move |window, _cx| {
            if update_blocks_close(_cx) {
                return false;
            }
            if let Some(lifecycle) = lifecycle.upgrade() {
                lifecycle.state.borrow_mut().capture(window.window_bounds());
                #[cfg(target_os = "windows")]
                if tray_enabled(_cx)
                    && lifecycle.tray_available(_cx)
                    && lifecycle.hide_windows(window)
                {
                    return false;
                }
            }
            true
        });
    }

    #[cfg(target_os = "windows")]
    fn hide_windows(&self, window: &Window) -> bool {
        if !self.hidden.get() {
            self.state.borrow_mut().capture(window.window_bounds());
            if !tray_windows::set_visible(window, false) {
                return false;
            }
            self.hidden.set(true);
            self.flush();
        }
        true
    }

    fn show(self: &Rc<Self>, cx: &mut App) {
        self.minimize_on_start.set(false);
        if let Some(handle) = self.window.get() {
            let _ = handle.update(cx, |_, window, _| {
                #[cfg(target_os = "windows")]
                if tray_windows::set_visible(window, true) {
                    self.hidden.set(false);
                }
                window.activate_window();
            });
        } else if self.hidden.replace(false) {
            self.open(cx);
        }
    }

    fn run(self: Rc<Self>, cx: &mut App) -> bool {
        let lifecycle = Rc::downgrade(&self);
        cx.on_window_closed(move |cx, id| {
            let Some(lifecycle) = lifecycle.upgrade() else {
                return;
            };
            if lifecycle
                .window
                .get()
                .is_none_or(|window| window.window_id() != id)
            {
                return;
            }
            lifecycle.window.set(None);
            lifecycle.minimize_on_start.set(false);
            lifecycle.flush();
            // This observer also covers app chrome calling remove_window(), and
            // runs before GPUI applies its last-window quit policy.
            #[cfg(target_os = "windows")]
            let keep_running = tray_enabled(cx);
            #[cfg(not(target_os = "windows"))]
            let keep_running = true;
            if keep_running && lifecycle.tray_available(cx) {
                lifecycle.hidden.set(true);
                cx.set_quit_mode(QuitMode::Explicit);
            }
        })
        .detach();
        let lifecycle = Rc::downgrade(&self);
        cx.on_app_quit(move |_| {
            if let Some(lifecycle) = lifecycle.upgrade() {
                lifecycle.flush();
                #[cfg(target_os = "windows")]
                lifecycle.windows_tray.borrow_mut().take();
            }
            std::future::ready(())
        })
        .detach();
        if !self.open(cx) {
            return false;
        }
        #[cfg(target_os = "linux")]
        self.sync_tray(cx);
        #[cfg(target_os = "windows")]
        match tray_windows::Tray::start() {
            Ok(tray) => *self.windows_tray.borrow_mut() = Some(tray),
            Err(error) => eprintln!("Windows tray unavailable: {error}"),
        }
        // App-owned, not tied to a window: launcher and tray still work while hidden.
        cx.spawn(async move |cx| {
            let mut ticks = 0;
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                if !cx.update(|cx| {
                    #[cfg(target_os = "linux")]
                    {
                        self.sync_tray(cx);
                    }
                    #[cfg(target_os = "linux")]
                    for action in self
                        .tray
                        .borrow()
                        .as_ref()
                        .into_iter()
                        .flat_map(tray::Tray::actions)
                    {
                        match action {
                            tray::Action::Show => self.show(cx),
                            tray::Action::Hide
                                if self.tray_available(cx) && !update_blocks_close(cx) =>
                            {
                                if let Some(handle) = self.window.get() {
                                    let _ = handle.update(cx, |_, window, _| {
                                        self.state.borrow_mut().capture(window.window_bounds());
                                        window.remove_window();
                                    });
                                }
                            }
                            tray::Action::Hide => {}
                            tray::Action::Quit => {
                                if !update_blocks_close(cx) {
                                    cx.quit();
                                    return false;
                                }
                            }
                        }
                    }
                    #[cfg(target_os = "windows")]
                    {
                        let action = self
                            .windows_tray
                            .borrow_mut()
                            .as_mut()
                            .and_then(tray_windows::Tray::poll);
                        match action {
                            Some(tray_windows::Action::Show) => self.show(cx),
                            Some(tray_windows::Action::Hide)
                                if self.tray_available(cx) && !update_blocks_close(cx) =>
                            {
                                if let Some(handle) = self.window.get() {
                                    let _ =
                                        handle.update(cx, |_, window, _| self.hide_windows(window));
                                }
                            }
                            Some(tray_windows::Action::Quit) => {
                                if !update_blocks_close(cx) {
                                    cx.quit();
                                    return false;
                                }
                            }
                            _ => {}
                        }
                        if self.hidden.get() && !self.tray_available(cx) {
                            self.show(cx);
                        }
                    }
                    if let Some(instance) = &self.instance {
                        let mut activate = false;
                        instance.drain(|link| {
                            activate = true;
                            if let Some(link) = link {
                                cx.set_global(PendingDeepLink(Some(link)));
                            }
                        });
                        if activate {
                            self.show(cx);
                        }
                    }
                    if let Some(handle) = self.window.get() {
                        let _ = handle.update(cx, |_, window, _| {
                            if !self.hidden.get() {
                                self.state.borrow_mut().capture(window.window_bounds());
                            }
                        });
                    } else if self.hidden.get() && !self.tray_available(cx) {
                        self.show(cx);
                    }
                    ticks = (ticks + 1) % 10;
                    if ticks == 0 {
                        self.flush();
                    }
                    true
                }) {
                    break;
                }
            }
        })
        .detach();
        true
    }
}

#[cfg(not(target_os = "windows"))]
fn config_home() -> io::Result<PathBuf> {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|path| path.join(".config"))
        })
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no XDG config directory"))
}

#[cfg(target_os = "windows")]
pub(crate) fn config_home() -> io::Result<PathBuf> {
    env::var_os("APPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no roaming AppData directory"))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no parent directory"))?;
    fs::create_dir_all(dir)?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    file.write_all(bytes)?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{is_tiling_desktop, should_start_minimized};

    #[test]
    fn recognizes_tiling_session_names() {
        for desktop in ["Hyprland", "omarchy", "wlroots:Sway", "niri", "i3"] {
            assert!(is_tiling_desktop(desktop), "{desktop}");
        }
        for desktop in ["", "GNOME", "ubuntu:GNOME", "KDE", "not-hyprland"] {
            assert!(!is_tiling_desktop(desktop), "{desktop}");
        }
    }

    #[test]
    fn startup_minimizes_only_when_requested_supported_and_not_opening_a_link() {
        assert!(should_start_minimized(true, true, false));
        assert!(!should_start_minimized(true, true, true));
        assert!(!should_start_minimized(true, false, false));
        assert!(!should_start_minimized(false, true, false));
    }
}
