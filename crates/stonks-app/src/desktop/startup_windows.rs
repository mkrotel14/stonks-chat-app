use super::APP_ID;
use std::{env, io, os::windows::ffi::OsStrExt, path::Path, ptr};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
    System::Registry::{
        RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_ANY,
    },
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

pub(crate) fn starts_with_computer() -> io::Result<bool> {
    let key = wide(RUN_KEY);
    let name = wide(APP_ID);
    let mut size = 0;
    // SAFETY: NUL-terminated names and a writable size pointer; no data is read.
    match unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_ANY,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut size,
        )
    } {
        ERROR_SUCCESS => Ok(true),
        ERROR_FILE_NOT_FOUND => Ok(false),
        status => Err(io::Error::from_raw_os_error(status as i32)),
    }
}

pub(crate) fn set_start_with_computer(enabled: bool) -> io::Result<()> {
    let key = wide(RUN_KEY);
    let name = wide(APP_ID);
    let status = if enabled {
        let command = run_command(&env::current_exe()?)?;
        // SAFETY: name and key are terminated; command contains the REG_SZ terminator.
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                REG_SZ,
                command.as_ptr().cast(),
                (command.len() * 2) as u32,
            )
        }
    } else {
        // SAFETY: name and key are NUL-terminated and live for the call.
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) }
    };
    match status {
        ERROR_SUCCESS => Ok(()),
        ERROR_FILE_NOT_FOUND if !enabled => Ok(()),
        status => Err(io::Error::from_raw_os_error(status as i32)),
    }
}

fn run_command(executable: &Path) -> io::Result<Vec<u16>> {
    let path: Vec<_> = executable.as_os_str().encode_wide().collect();
    // The Run key limits command lines to 260 characters, including quoting.
    if !executable.is_absolute() || path.len() + 2 > 260 || path.iter().any(|&c| c < 32 || c == 34)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsupported startup executable path",
        ));
    }
    Ok([34].into_iter().chain(path).chain([34, 0]).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_command_quotes_absolute_executable() {
        assert_eq!(
            run_command(Path::new(r"C:\Program Files\Stonks\stonks.exe")).unwrap(),
            wide("\"C:\\Program Files\\Stonks\\stonks.exe\"")
        );
        assert!(run_command(Path::new("stonks.exe")).is_err());
    }
}
