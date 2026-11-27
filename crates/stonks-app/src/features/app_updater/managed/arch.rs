//! Arch package ownership, version ordering and installation.

use super::{
    linux::{authorize, stdout},
    Candidate, Installed, Kind, Result, MANAGED,
};
use std::{path::Path, process::Command};

const PACMAN: &str = "/usr/bin/pacman";

fn pacman() -> Command {
    let mut command = Command::new(PACMAN);
    command.env("LC_ALL", "C");
    command
}

pub(super) fn eligible() -> bool {
    std::env::var_os("APPIMAGE").is_none()
        && Path::new("/etc/arch-release").is_file()
        && std::env::current_exe().ok().as_deref() == Some(Path::new(MANAGED))
}

pub(super) fn installed() -> Result<Installed> {
    if !eligible() {
        return Err("This is not a managed Arch installation.".into());
    }
    let owner = stdout(pacman().args(["-Qoq", "--", MANAGED]).output())?;
    let name = owner.trim();
    if !matches!(name, "stonks" | "stonks-gpui") {
        return Err("The executable is not owned by a supported Stonks package.".into());
    }
    let query = stdout(pacman().args(["-Q", "--", name]).output())?;
    let version = query
        .trim()
        .strip_prefix(&format!("{name} "))
        .filter(|version| !version.is_empty() && !version.contains(char::is_whitespace))
        .ok_or("Could not determine the installed package version.")?;
    Ok(Installed {
        kind: Kind::Arch,
        name: name.into(),
        version: version.into(),
    })
}

pub(super) fn newer(candidate: &str, installed: &str) -> Result<bool> {
    match stdout(
        Command::new("/usr/bin/vercmp")
            .args([candidate, installed])
            .output(),
    )?
    .trim()
    {
        "1" => Ok(true),
        "0" | "-1" => Ok(false),
        _ => Err("Could not compare Arch package versions.".into()),
    }
}

pub(super) fn verify_package(candidate: &Candidate, info: &str) -> Result<()> {
    for (key, expected) in [
        ("Name", candidate.installed.name.as_str()),
        ("Version", &candidate.version),
        ("Architecture", "x86_64"),
    ] {
        if !info.lines().any(|line| {
            line.split_once(':')
                .is_some_and(|(field, value)| field.trim_end() == key && value.trim() == expected)
        }) {
            return Err(format!("Downloaded package has an unexpected {key}."));
        }
    }
    Ok(())
}

pub(super) fn install(candidate: &Candidate, path: &Path, installing: impl FnOnce()) -> Result<()> {
    let info = stdout(pacman().args(["-Qip", "--"]).arg(&path).output())?;
    verify_package(candidate, &info)?;
    super::still_installed(candidate)?;
    installing();
    authorize(PACMAN, &["-U", "--noconfirm", "--"], path)?;
    if installed()?.version != candidate.version {
        return Err("Pacman did not report the expected installed package version.".into());
    }
    Ok(())
}
