//! The Fedora 44 x86-64 GPUI RPM only, not the legacy Tauri package.

use super::{
    linux::{authorize, stdout},
    rpm_version, Candidate, Installed, Kind, Result, MANAGED,
};
use std::{path::Path, process::Command};

pub(super) const DNF: &str = "/usr/bin/dnf";
const RPM: &str = "/usr/bin/rpm";
const NAME: &str = "stonks-gpui";
const QUERY: &str = "%{NAME}\t%{EPOCHNUM}\t%{VERSION}-%{RELEASE}\t%{ARCH}\n";

fn fedora_44(os_release: &str) -> bool {
    let field = |key| {
        os_release.lines().find_map(|line| {
            let (name, value) = line.split_once('=')?;
            (name == key).then_some(value.trim_matches(['\'', '"']))
        })
    };
    field("ID") == Some("fedora") && field("VERSION_ID") == Some("44")
}

fn identity(info: &str) -> Result<Installed> {
    let fields: Vec<_> = info.trim_end_matches('\n').split('\t').collect();
    if let [NAME, "0", version, "x86_64"] = fields.as_slice() {
        if rpm_version(version).is_some() {
            return Ok(Installed {
                kind: Kind::Rpm,
                name: NAME.into(),
                version: (*version).into(),
            });
        }
    }
    Err("Unexpected Fedora package name, epoch, version, release, or architecture.".into())
}

pub(super) fn installed() -> Result<Installed> {
    if std::env::var_os("APPIMAGE").is_some()
        || std::env::current_exe().ok().as_deref() != Some(Path::new(MANAGED))
        || !std::fs::read_to_string("/etc/os-release").is_ok_and(|info| fedora_44(&info))
    {
        return Err("This is not a managed Fedora 44 installation.".into());
    }
    identity(&stdout(
        Command::new(RPM)
            .args(["-qf", "--qf", QUERY, "--", MANAGED])
            .output(),
    )?)
}

fn verify_package(candidate: &Candidate, info: &str, files: &str) -> Result<()> {
    if identity(info)?.version != candidate.version || !files.lines().any(|file| file == MANAGED) {
        return Err(
            "The Fedora package does not match the selected version and executable.".into(),
        );
    }
    Ok(())
}

pub(super) fn install(candidate: &Candidate, path: &Path, installing: impl FnOnce()) -> Result<()> {
    let info = stdout(
        Command::new(RPM)
            .args(["-qp", "--qf", QUERY, "--"])
            .arg(path)
            .output(),
    )?;
    let files = stdout(Command::new(RPM).args(["-qpl", "--"]).arg(path).output())?;
    verify_package(candidate, &info, &files)?;
    super::still_installed(candidate)?;
    installing();
    authorize(DNF, &["install", "-y"], path)?;
    if installed()?.version != candidate.version {
        return Err("RPM did not report the expected installed package version.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_fedora_release_and_full_rpm_identity() {
        assert!(fedora_44("ID=fedora\nVERSION_ID=\"44\"\n"));
        assert!(!fedora_44("ID=nobara\nID_LIKE=fedora\nVERSION_ID=44"));
        assert!(!fedora_44("ID=fedora\nVERSION_ID=43"));
        let info = "stonks-gpui\t0\t4.0.3-1.fc44\tx86_64\n";
        assert_eq!(identity(info).unwrap().version, "4.0.3-1.fc44");
        for (good, bad) in [
            ("stonks-gpui", "stonks-chat"),
            ("\t0\t", "\t1\t"),
            ("fc44", "fc43"),
            ("x86_64", "aarch64"),
        ] {
            assert!(identity(&info.replace(good, bad)).is_err());
        }
        assert!(identity(&info.repeat(2)).is_err());
        let candidate = Candidate {
            installed: identity(&info.replace("4.0.3", "4.0.2")).unwrap(),
            version: "4.0.3-1.fc44".into(),
            name: String::new(),
            url: String::new(),
            size: 0,
            digest: String::new(),
        };
        assert!(verify_package(&candidate, info, "/usr/bin/stonks-app\n").is_ok());
        assert!(verify_package(&candidate, info, "/usr/bin/stonks\n").is_err());
        assert!(verify_package(&candidate, &info.replace("4.0.3", "4.0.4"), MANAGED).is_err());
    }
}
