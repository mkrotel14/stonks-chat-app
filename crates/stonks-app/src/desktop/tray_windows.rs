//! Explorer tray and process-local balloon activation. All HWNDs are UI-thread owned.
use futures_channel::oneshot;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    cell::Cell,
    io, mem, ptr,
    sync::Mutex,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::{Shell::*, WindowsAndMessaging::*},
};

const CALLBACK: u32 = WM_APP + 1;
const CLASS: *const u16 = windows_sys::core::w!("com.rdxlabs.stonks.gpui.tray");
static BALLOONS: Mutex<Option<mpsc::Sender<Balloon>>> = Mutex::new(None);

pub(crate) struct Balloon {
    pub title: String,
    pub body: String,
    pub current: Box<dyn Fn() -> bool + Send>,
    pub shown: Option<oneshot::Sender<()>>,
    pub clicked: oneshot::Sender<bool>,
}

pub(crate) fn notify(balloon: Balloon) -> bool {
    BALLOONS
        .lock()
        .ok()
        .and_then(|sender| sender.as_ref().map(|s| s.try_send(balloon).is_ok()))
        .unwrap_or(false)
}

#[derive(Clone, Copy)]
pub(super) enum Action {
    Show,
    Hide,
    Quit,
}

thread_local! {
    static ACTION: Cell<Option<Action>> = const { Cell::new(None) };
    static BALLOON_EVENT: Cell<Option<(u32, bool)>> = const { Cell::new(None) };
    static RESTARTED: Cell<bool> = const { Cell::new(false) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
    static ICON_ID: Cell<u32> = const { Cell::new(0) };
}

pub(super) struct Tray {
    data: NOTIFYICONDATAW,
    available: bool,
    next_id: u32,
    requests: mpsc::Receiver<Balloon>,
    active: Option<(Instant, Balloon)>,
}

impl Tray {
    pub(super) fn start() -> io::Result<Self> {
        // SAFETY: the registered callback and all window/icon operations live on the GPUI thread.
        unsafe {
            let module = GetModuleHandleW(ptr::null());
            let class = WNDCLASSW {
                lpfnWndProc: Some(receive),
                hInstance: module,
                lpszClassName: CLASS,
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 {
                return Err(io::Error::last_os_error());
            }
            TASKBAR_CREATED.set(RegisterWindowMessageW(windows_sys::core::w!(
                "TaskbarCreated"
            )));
            // Explorer broadcasts TaskbarCreated only to top-level windows.
            let hwnd = CreateWindowExW(
                0,
                CLASS,
                CLASS,
                WS_POPUP,
                0,
                0,
                0,
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                module,
                ptr::null(),
            );
            if hwnd.is_null() {
                let error = io::Error::last_os_error();
                UnregisterClassW(CLASS, module);
                return Err(error);
            }
            let (sender, requests) = mpsc::channel(1);
            if let Ok(mut slot) = BALLOONS.lock() {
                *slot = Some(sender);
            }
            let mut tray = Self {
                data: NOTIFYICONDATAW {
                    cbSize: mem::size_of::<NOTIFYICONDATAW>() as u32,
                    hWnd: hwnd,
                    hIcon: LoadIconW(ptr::null_mut(), IDI_APPLICATION),
                    uCallbackMessage: CALLBACK,
                    ..Default::default()
                },
                available: false,
                next_id: 1,
                requests,
                active: None,
            };
            tray.install();
            Ok(tray)
        }
    }

    fn install(&mut self) {
        self.data.uID = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        copy_text(&mut self.data.szTip, "Stonks Chat");
        // SAFETY: data is initialized and the owned tray window is still alive.
        self.available = unsafe { Shell_NotifyIconW(NIM_ADD, &self.data) != 0 };
        ICON_ID.set(if self.available { self.data.uID } else { 0 });
    }

    fn remove(&mut self) {
        ICON_ID.set(0);
        if self.available {
            // SAFETY: the icon is identified by the live window and its current ID.
            unsafe {
                Shell_NotifyIconW(NIM_DELETE, &self.data);
            }
            self.available = false;
        }
        self.active.take();
    }

    pub(super) fn available(&self) -> bool {
        self.available && !RESTARTED.get()
    }

    pub(super) fn poll(&mut self) -> Option<Action> {
        if RESTARTED.replace(false) {
            self.available = false;
            self.remove();
            self.install();
        }
        let event = BALLOON_EVENT.take();
        if let Some((_, clicked)) = event.filter(|(id, _)| *id == self.data.uID) {
            if let Some((_, request)) = self.active.take() {
                let activated = clicked && !request.clicked.is_canceled() && (request.current)();
                if request.clicked.send(activated).is_ok() && activated {
                    ACTION.set(Some(Action::Show));
                }
            }
            // Keep late callbacks from a retired balloon from activating a new route.
            self.remove();
            self.install();
        } else if self.active.as_ref().is_some_and(|(start, request)| {
            request.clicked.is_canceled()
                || !(request.current)()
                || start.elapsed() >= Duration::from_secs(60)
        }) {
            self.remove();
            self.install();
        }
        if self.active.is_none() {
            if let Ok(mut request) = self.requests.try_recv() {
                if self.available() && !request.clicked.is_canceled() && (request.current)() {
                    self.data.uFlags = NIF_INFO;
                    self.data.dwInfoFlags = NIIF_INFO | NIIF_NOSOUND | NIIF_RESPECT_QUIET_TIME;
                    copy_text(&mut self.data.szInfoTitle, &request.title);
                    copy_text(&mut self.data.szInfo, &request.body);
                    // SAFETY: both fixed-size buffers are terminated and the icon is installed.
                    if unsafe { Shell_NotifyIconW(NIM_MODIFY, &self.data) } != 0 {
                        if let Some(shown) = request.shown.take() {
                            let _ = shown.send(());
                        }
                        self.active = Some((Instant::now(), request));
                    }
                }
            }
        }
        ACTION.take()
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        if let Ok(mut slot) = BALLOONS.lock() {
            slot.take();
        }
        self.remove();
        // SAFETY: the creating thread owns the window and registration.
        unsafe {
            DestroyWindow(self.data.hWnd);
            UnregisterClassW(CLASS, GetModuleHandleW(ptr::null()));
        }
    }
}

fn copy_text(buffer: &mut [u16], text: &str) {
    buffer.fill(0);
    let mut offset = 0;
    for ch in text.chars().filter(|ch| !ch.is_control() || *ch == '\n') {
        let mut units = [0; 2];
        let units = ch.encode_utf16(&mut units);
        if offset + units.len() >= buffer.len() {
            break;
        }
        buffer[offset..offset + units.len()].copy_from_slice(units);
        offset += units.len();
    }
}

unsafe extern "system" fn receive(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message != 0 && message == TASKBAR_CREATED.get() {
        RESTARTED.set(true);
        return 0;
    }
    if message == CALLBACK && wparam == ICON_ID.get() as usize && wparam != 0 {
        match lparam as u32 {
            WM_LBUTTONUP => ACTION.set(Some(Action::Show)),
            WM_RBUTTONUP => show_menu(hwnd),
            NIN_BALLOONUSERCLICK => BALLOON_EVENT.set(Some((wparam as u32, true))),
            NIN_BALLOONHIDE | NIN_BALLOONTIMEOUT => {
                if BALLOON_EVENT.get().is_none() {
                    BALLOON_EVENT.set(Some((wparam as u32, false)));
                }
            }
            _ => {}
        }
        return 0;
    }
    // SAFETY: forward the unhandled message unchanged.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

fn show_menu(hwnd: HWND) {
    // SAFETY: called on the tray window thread; the temporary menu is destroyed here.
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        AppendMenuW(
            menu,
            MF_STRING,
            1,
            windows_sys::core::w!("Show Stonks Chat"),
        );
        AppendMenuW(
            menu,
            MF_STRING,
            2,
            windows_sys::core::w!("Hide Stonks Chat"),
        );
        AppendMenuW(menu, MF_STRING, 3, windows_sys::core::w!("Quit"));
        let mut point = POINT::default();
        if GetCursorPos(&mut point) != 0 {
            SetForegroundWindow(hwnd);
            let command = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                point.x,
                point.y,
                0,
                hwnd,
                ptr::null(),
            );
            PostMessageW(hwnd, WM_NULL, 0, 0);
            match command {
                1 => ACTION.set(Some(Action::Show)),
                2 => ACTION.set(Some(Action::Hide)),
                3 => ACTION.set(Some(Action::Quit)),
                _ => {}
            }
        }
        DestroyMenu(menu);
    }
}

pub(super) fn set_visible(window: &gpui_kit::Window, visible: bool) -> bool {
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return false;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return false;
    };
    let hwnd = handle.hwnd.get() as HWND;
    // SAFETY: GPUI owns this HWND for the duration of the call on the UI thread.
    unsafe {
        if visible {
            ShowWindow(
                hwnd,
                if IsIconic(hwnd) != 0 {
                    SW_RESTORE
                } else {
                    SW_SHOW
                },
            );
            SetForegroundWindow(hwnd);
        } else {
            ShowWindow(hwnd, SW_HIDE);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::copy_text;

    #[test]
    fn shell_text_is_terminated_without_splitting_utf16() {
        let mut field = [0xFFFF; 4];
        copy_text(&mut field, "ab😀");
        assert_eq!(field, [97, 98, 0, 0]);
        copy_text(&mut field, "a\0😀b");
        assert_eq!(&field, &[97, 0xD83D, 0xDE00, 0]);
    }
}
