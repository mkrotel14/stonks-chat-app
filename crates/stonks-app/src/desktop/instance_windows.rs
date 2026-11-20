use super::{
    deep_links::{validate, MAX_DEEP_LINK_BYTES},
    DeepLink, APP_ID,
};
use std::{
    cell::Cell,
    io,
    os::windows::io::{FromRawHandle, OwnedHandle},
    ptr, thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, WPARAM},
    System::{
        DataExchange::COPYDATASTRUCT, LibraryLoader::GetModuleHandleW, Threading::CreateMutexW,
    },
    UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, CreateWindowExW, DefWindowProcW, DestroyWindow, FindWindowExW,
        GetWindowThreadProcessId, RegisterClassW, SendMessageTimeoutW, UnregisterClassW,
        HWND_MESSAGE, SMTO_ABORTIFHUNG, SMTO_BLOCK, WM_COPYDATA, WNDCLASSW,
    },
};

const ACTIVATION: usize = 1;
thread_local! {
    // The GPUI UI thread owns both the receiver window and this bounded inbox.
    static PENDING: Cell<Option<Option<DeepLink>>> = const { Cell::new(None) };
}

pub(super) struct Instance {
    window: HWND,
    class: Vec<u16>,
    _mutex: OwnedHandle,
}

impl Instance {
    pub(super) fn acquire(link: Option<&str>) -> io::Result<Option<Self>> {
        let name = wide(&format!("Local\\{APP_ID}.instance"));
        let class = wide(&format!("{APP_ID}.activation"));
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            // SAFETY: the name is NUL-terminated; the handle keeps the named object alive.
            let raw = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };
            let status = unsafe { GetLastError() };
            if raw.is_null() {
                return Err(io::Error::from_raw_os_error(status as i32));
            }
            // SAFETY: a successful CreateMutexW returns a newly owned handle.
            let mutex = unsafe { OwnedHandle::from_raw_handle(raw) };
            if status != ERROR_ALREADY_EXISTS {
                return Self::listen(class, mutex).map(Some);
            }
            drop(mutex);
            // SAFETY: the class name remains valid across this synchronous lookup.
            let window = unsafe {
                FindWindowExW(HWND_MESSAGE, ptr::null_mut(), class.as_ptr(), ptr::null())
            };
            if !window.is_null() && forward(window, link) {
                return Ok(None);
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "primary did not accept activation",
                ));
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn listen(class: Vec<u16>, mutex: OwnedHandle) -> io::Result<Self> {
        // SAFETY: register a callback and create its message-only window on the same
        // thread that will run GPUI's message loop. Win32 copies the class name.
        unsafe {
            let module = GetModuleHandleW(ptr::null());
            if module.is_null() {
                return Err(io::Error::last_os_error());
            }
            let descriptor = WNDCLASSW {
                lpfnWndProc: Some(receive),
                hInstance: module,
                lpszClassName: class.as_ptr(),
                ..Default::default()
            };
            if RegisterClassW(&descriptor) == 0 {
                return Err(io::Error::last_os_error());
            }
            let window = CreateWindowExW(
                0,
                class.as_ptr(),
                ptr::null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                ptr::null_mut(),
                module,
                ptr::null(),
            );
            if window.is_null() {
                let error = io::Error::last_os_error();
                UnregisterClassW(class.as_ptr(), module);
                return Err(error);
            }
            Ok(Self {
                window,
                class,
                _mutex: mutex,
            })
        }
    }

    pub(super) fn drain(&self, mut accept: impl FnMut(Option<DeepLink>)) {
        if let Some(link) = PENDING.with(Cell::take) {
            accept(link);
        }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: teardown runs on the owning GUI thread, before dropping the mutex.
        unsafe {
            DestroyWindow(self.window);
            UnregisterClassW(self.class.as_ptr(), GetModuleHandleW(ptr::null()));
        }
        PENDING.with(Cell::take);
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

fn forward(window: HWND, link: Option<&str>) -> bool {
    let bytes = link.unwrap_or("").as_bytes();
    let data = COPYDATASTRUCT {
        dwData: ACTIVATION,
        cbData: bytes.len() as u32,
        lpData: bytes.as_ptr().cast_mut().cast(),
    };
    let mut accepted = 0;
    // SAFETY: WM_COPYDATA marshals the descriptor and bytes during the send;
    // both buffers and the result pointer are valid for the call.
    let sent = unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(window, &mut pid);
        if pid != 0 {
            AllowSetForegroundWindow(pid);
        }
        SendMessageTimeoutW(
            window,
            WM_COPYDATA,
            0,
            &data as *const _ as LPARAM,
            SMTO_ABORTIFHUNG | SMTO_BLOCK,
            200,
            &mut accepted,
        )
    };
    sent != 0 && accepted == 1
}

unsafe extern "system" fn receive(
    window: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg != WM_COPYDATA {
        // SAFETY: forward the original Win32 callback arguments unchanged.
        return unsafe { DefWindowProcW(window, msg, wparam, lparam) };
    }
    // SAFETY: Win32 supplies the COPYDATASTRUCT and keeps its data alive for this callback.
    let Some(data) = (unsafe { (lparam as *const COPYDATASTRUCT).as_ref() }) else {
        return 0;
    };
    if data.dwData != ACTIVATION || data.cbData as usize > MAX_DEEP_LINK_BYTES {
        return 0;
    }
    let link = if data.cbData == 0 {
        None
    } else {
        if data.lpData.is_null() {
            return 0;
        }
        // SAFETY: the non-null, bounded payload is borrowed only inside the callback.
        let bytes = unsafe { std::slice::from_raw_parts(data.lpData.cast(), data.cbData as usize) };
        let Some(link) = std::str::from_utf8(bytes).ok().and_then(validate) else {
            return 0;
        };
        Some(link)
    };
    queue(link);
    1
}

fn queue(link: Option<DeepLink>) {
    PENDING.with(|pending| {
        let previous = pending.take().flatten();
        pending.set(Some(link.or(previous)));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_activation_preserves_latest_valid_link() {
        queue(validate("stonks://invite/first"));
        let latest = validate("stonks://invite/latest").unwrap();
        queue(Some(latest.clone()));
        queue(None);
        assert_eq!(PENDING.with(Cell::take), Some(Some(latest)));
        queue(None);
        assert_eq!(PENDING.with(Cell::take), Some(None));
    }
}
