use super::{arch, rpm, Candidate, Installed, Kind, Result, MANAGED};
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
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

pub(super) fn running_executable() -> Result<PathBuf> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    if std::env::var_os("APPIMAGE").is_some() || executable != Path::new(MANAGED) {
        return Err("This is not a managed Linux installation.".into());
    }
    Ok(executable)
}

fn installed_at(kind: Kind, executable: &Path) -> Result<Installed> {
    match kind {
        Kind::Arch if arch::supported_distro() => arch::installed_at(executable),
        Kind::Rpm if rpm::supported_distro() => rpm::installed_at(executable),
        _ => Err("This is not a supported managed Linux installation.".into()),
    }
}

pub(super) fn installed(kind: Kind) -> Result<Installed> {
    installed_at(kind, &running_executable()?)
}

pub(super) fn install(
    candidate: &Candidate,
    program: &str,
    args: &[&str],
    path: &Path,
    installing: impl FnOnce(),
) -> Result<PathBuf> {
    let kind = candidate.kind();
    // Validate and save the path before the transaction unlinks the running inode.
    let executable = running_executable()?;
    super::verify_unchanged(candidate, installed_at(kind, &executable)?)?;
    installing();
    authorize(program, args, path)?;
    verify_replacement(candidate, &executable, |path| installed_at(kind, path))?;
    Ok(executable)
}

fn verify_replacement(
    candidate: &Candidate,
    executable: &Path,
    query: impl FnOnce(&Path) -> Result<Installed>,
) -> Result<()> {
    let updated = query(executable)?;
    if updated.kind != candidate.kind()
        || updated.name != candidate.installed.name
        || updated.version != candidate.version
    {
        return Err(
            "The package manager did not report the expected installed package and version.".into(),
        );
    }
    if !std::fs::symlink_metadata(executable)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    {
        return Err("The updated executable is missing or not executable.".into());
    }
    Ok(())
}

fn authorize(program: &str, args: &[&str], path: &Path) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::MetadataExt};

    #[test]
    fn verifies_the_replacement_at_the_saved_executable_path() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("stonks-app");
        fs::write(&executable, b"old").unwrap();
        let running = fs::File::open(&executable).unwrap();
        let replacement = directory.path().join("replacement");
        fs::write(&replacement, b"new").unwrap();
        fs::set_permissions(&replacement, fs::Permissions::from_mode(0o755)).unwrap();
        fs::rename(replacement, &executable).unwrap();
        assert_eq!(running.metadata().unwrap().nlink(), 0);

        for (kind, name, before, after) in [
            (Kind::Arch, "stonks", "4.0.1-1", "4.0.1-2"),
            (Kind::Rpm, "stonks-gpui", "4.0.1-1.fc44", "4.0.1-2.fc44"),
        ] {
            let candidate = Candidate {
                installed: Installed {
                    kind,
                    name: name.into(),
                    version: before.into(),
                },
                version: after.into(),
                name: String::new(),
                url: String::new(),
                size: 0,
                digest: String::new(),
                release_name: None,
                release_notes: None,
            };
            let updated = Installed {
                version: after.into(),
                ..candidate.installed.clone()
            };
            verify_replacement(&candidate, &executable, |path| {
                assert_eq!(path, executable);
                assert_eq!(fs::read(path).unwrap(), b"new");
                Ok(updated.clone())
            })
            .unwrap();
            for wrong_package in [
                candidate.installed.clone(),
                Installed {
                    name: "other".into(),
                    ..updated.clone()
                },
            ] {
                assert!(
                    verify_replacement(&candidate, &executable, |_| Ok(wrong_package)).is_err()
                );
            }
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o644)).unwrap();
            assert!(verify_replacement(&candidate, &executable, |_| Ok(updated)).is_err());
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}
