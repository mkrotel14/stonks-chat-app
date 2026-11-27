//! Arch package ownership, version ordering and installation.

use super::{
    linux::{self, stdout},
    Candidate, Installed, Kind, Result,
};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

const PACMAN: &str = "/usr/bin/pacman";

fn pacman() -> Command {
    let mut command = Command::new(PACMAN);
    command.env("LC_ALL", "C");
    command
}

pub(super) fn supported_distro() -> bool {
    Path::new("/etc/arch-release").is_file()
}

pub(super) fn installed_at(executable: &Path) -> Result<Installed> {
    let owner = stdout(pacman().args(["-Qoq", "--"]).arg(executable).output())?;
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

pub(super) fn install(
    candidate: &Candidate,
    path: &Path,
    installing: impl FnOnce(),
) -> Result<PathBuf> {
    let info = stdout(pacman().args(["-Qip", "--"]).arg(path).output())?;
    verify_package(candidate, &info)?;
    linux::install(
        candidate,
        PACMAN,
        &["-U", "--noconfirm", "--"],
        path,
        installing,
    )
}
