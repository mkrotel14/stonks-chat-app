use super::{Kind, Result};
use std::{
    path::Path,
    process::{Command, Output, Stdio},
};

const PKEXEC: &str = "/usr/bin/pkexec";

pub(super) fn preflight(kind: Kind) -> Result<()> {
    // Do not silently skip authorization when the GUI runs as root.
    if unsafe { libc::geteuid() } == 0 {
        return Err("Run Stonks Chat as a normal user to install updates.".into());
    }
    if !Path::new(PKEXEC).is_file() {
        return Err(
            "Install polkit and run a desktop authentication agent to use the updater.".into(),
        );
    }
    if kind == Kind::Rpm && !Path::new(super::rpm::DNF).is_file() {
        return Err("DNF is required to install Fedora updates.".into());
    }
    Ok(())
}

pub(super) fn authorize(program: &str, args: &[&str], path: &Path) -> Result<()> {
    let output = Command::new(PKEXEC)
        .args(["--disable-internal-agent", program])
        .args(args)
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| error.to_string())?;
    match output.status.code() {
        Some(126) => Err("Authorization was canceled.".into()),
        Some(127) => Err(format!(
            "Authorization failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        _ => stdout(Ok(output)).map(|_| ()),
    }
}

pub(super) fn stdout(output: std::io::Result<Output>) -> Result<String> {
    let output = output.map_err(|error| error.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("{}: {}", output.status, stderr.trim()));
    }
    String::from_utf8(output.stdout).map_err(|error| error.to_string())
}
