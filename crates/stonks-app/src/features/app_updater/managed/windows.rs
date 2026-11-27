//! Update only the registered per-user GPUI Inno Setup installation.

use super::{Candidate, Installed, Kind, Result};
use std::{
    ffi::OsString,
    os::windows::{ffi::OsStringExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    ptr,
};
use windows_sys::Win32::{
    Foundation::ERROR_SUCCESS,
    System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ},
    System::Threading::CREATE_NO_WINDOW,
};

const UNINSTALL: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Uninstall\com.rdxlabs.stonks.gpui_is1";
const EXE: &str = "stonks-app.exe";

fn registered(name: &str) -> Result<OsString> {
    let wide = |value: &str| value.encode_utf16().chain([0]).collect::<Vec<_>>();
    let (key, value) = (wide(UNINSTALL), wide(name));
    let mut buffer = vec![0u16; 32_768];
    let mut bytes = (buffer.len() * 2) as u32;
    // SAFETY: NUL-terminated names; buffer has capacity for the supplied byte count.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(status as i32).to_string());
    }
    let units = &buffer[..bytes as usize / 2];
    let units = units
        .strip_suffix(&[0])
        .filter(|s| !s.is_empty() && !s.contains(&0))
        .ok_or("Invalid GPUI installer registration.")?;
    Ok(OsString::from_wide(units))
}

pub(super) fn install_dir() -> Result<PathBuf> {
    let directory = PathBuf::from(registered("Inno Setup: App Path")?);
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    if !directory.is_absolute()
        || executable
            .file_name()
            .and_then(|name| name.to_str())
            .is_none_or(|name| !name.eq_ignore_ascii_case(EXE))
        || std::fs::canonicalize(directory.join(EXE)).map_err(|error| error.to_string())?
            != std::fs::canonicalize(executable).map_err(|error| error.to_string())?
    {
        return Err("This is not the registered GPUI installation.".into());
    }
    Ok(directory)
}

pub(super) fn installed() -> Result<Installed> {
    install_dir()?;
    let version = registered("DisplayVersion")?;
    if version != super::super::CURRENT_VERSION {
        return Err("The GPUI installer registration does not match the running version.".into());
    }
    Ok(Installed {
        kind: Kind::Windows,
        name: "Stonks GPUI".into(),
        version: version.to_string_lossy().into_owned(),
    })
}

pub(super) fn launch(
    directory: tempfile::TempDir,
    path: &Path,
    candidate: &Candidate,
) -> Result<()> {
    Command::new(path)
        .arg("/UPDATE")
        .arg(format!("/UPDATEPID={}", std::process::id()))
        .arg(format!("/UPDATEFROM={}", candidate.installed.version))
        .current_dir(directory.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|error| format!("Could not launch the GPUI installer: {error}"))?;
    // Inno may reopen its original setup file after spawning the setup UI.
    // Keep this file through the handoff; dropping it here races Setup.
    let _ = directory.keep();
    Ok(())
}
