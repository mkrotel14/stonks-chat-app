//! Verified Arch updates from the standalone GPUI release repository.

use reqwest::{redirect::Policy, Client, Response};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
    time::Duration,
};

const PACMAN: &str = "/usr/bin/pacman";
const PKEXEC: &str = "/usr/bin/pkexec";
const MANAGED: &str = "/usr/bin/stonks-app";
const RELEASE_API: &str = "https://api.github.com/repos/mkrotel14/stonks-chat-app/releases/latest";
const DOWNLOAD_BASE: &str = "https://github.com/mkrotel14/stonks-chat-app/releases/download";
const MAX_PACKAGE: u64 = 512 * 1024 * 1024;
type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Installed {
    name: String,
    version: String,
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

#[derive(Clone)]
pub(crate) struct Candidate {
    installed: Installed,
    version: String,
    name: String,
    url: String,
    size: u64,
    digest: String,
}

impl Candidate {
    pub(crate) fn label(&self) -> String {
        format!(
            "{} {} → {}",
            self.installed.name, self.installed.version, self.version
        )
    }
}

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

fn installed() -> Result<Installed> {
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
        name: name.into(),
        version: version.into(),
    })
}

fn stdout(output: std::io::Result<Output>) -> Result<String> {
    let output = output.map_err(|error| error.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("{}: {}", output.status, stderr.trim()));
    }
    String::from_utf8(output.stdout).map_err(|error| error.to_string())
}

fn newer(candidate: &str, installed: &str) -> Result<bool> {
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

fn select(release: Release, installed: Installed) -> Result<Option<Candidate>> {
    let tag = &release.tag_name;
    let version = tag.strip_prefix(['v', 'V']).unwrap_or(tag);
    let numeric = |part: &str| !part.is_empty() && part.bytes().all(|c| c.is_ascii_digit());
    if release.draft
        || release.prerelease
        || version.split('.').count() != 3
        || !version.split('.').all(numeric)
    {
        return Err("Invalid stable GPUI release tag.".into());
    }
    let prefix = format!("{}-{version}-", installed.name);
    let mut matches = release.assets.iter().filter_map(|asset| {
        let revision = asset
            .name
            .strip_prefix(&prefix)?
            .strip_suffix("-x86_64.pkg.tar.zst")?;
        numeric(revision).then_some((asset, revision))
    });
    let Some((asset, revision)) = matches.next() else {
        if !newer(version, &installed.version)? {
            return Ok(None);
        }
        return Err(format!(
            "The latest GPUI release has no {} Arch package.",
            installed.name
        ));
    };
    if matches.next().is_some() {
        return Err("The GPUI release contains ambiguous Arch packages.".into());
    }
    let version = format!("{version}-{revision}");
    if !newer(&version, &installed.version)? {
        return Ok(None);
    }
    let url = format!("{DOWNLOAD_BASE}/{tag}/{}", asset.name);
    let digest = asset
        .digest
        .as_deref()
        .and_then(|value| value.strip_prefix("sha256:"))
        .filter(|hex| hex.len() == 64 && hex.bytes().all(|c| c.is_ascii_hexdigit()))
        .ok_or("The Arch package has no valid GitHub SHA-256 digest.")?;
    if asset.browser_download_url != url || !(1..=MAX_PACKAGE).contains(&asset.size) {
        return Err("Invalid Arch package URL or size in GitHub metadata.".into());
    }
    Ok(Some(Candidate {
        installed,
        version,
        name: asset.name.clone(),
        url,
        size: asset.size,
        digest: digest.to_ascii_lowercase(),
    }))
}

fn client(policy: Policy, timeout: u64) -> Result<Client> {
    Client::builder()
        .user_agent("Stonks-GPUI")
        .https_only(true)
        .redirect(policy)
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(timeout))
        .build()
        .map_err(|error| error.to_string())
}

fn response_ok(response: Response) -> Result<Response> {
    if response.status() != reqwest::StatusCode::OK {
        return Err(format!("GitHub returned HTTP {}.", response.status()));
    }
    Ok(response)
}

pub(super) async fn check() -> Result<Option<Candidate>> {
    let installed = installed()?;
    let mut response = response_ok(
        client(Policy::none(), 15)?
            .get(RELEASE_API)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|error| error.to_string())?,
    )?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err("GPUI release metadata is too large.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    select(
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?,
        installed,
    )
}

fn allowed_redirect(url: &url::Url) -> bool {
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(
            url.host_str(),
            Some("release-assets.githubusercontent.com" | "objects.githubusercontent.com")
        )
}

fn verify_download(candidate: &Candidate, size: u64, hash: Sha256) -> Result<()> {
    if size != candidate.size || format!("{:x}", hash.finalize()) != candidate.digest {
        return Err(
            "The downloaded package does not match its GitHub size and SHA-256 digest.".into(),
        );
    }
    Ok(())
}

fn verify_package(candidate: &Candidate, info: &str) -> Result<()> {
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

fn still_installed(candidate: &Candidate) -> Result<()> {
    if installed()? != candidate.installed {
        return Err("The installed package changed. Check for updates again.".into());
    }
    Ok(())
}

pub(super) async fn install(
    candidate: Candidate,
    installing: impl FnOnce() + Send + 'static,
) -> Result<()> {
    // Never let pkexec become a no-op because the GUI is already privileged.
    if unsafe { libc::geteuid() } == 0 {
        return Err("Run Stonks Chat as a normal user to install updates.".into());
    }
    if !Path::new(PKEXEC).is_file() {
        return Err(
            "Install polkit and run a desktop authentication agent to use the updater.".into(),
        );
    }
    still_installed(&candidate)?;
    let mut response = response_ok(
        client(
            Policy::custom(|attempt| {
                if attempt.previous().len() <= 3 && allowed_redirect(attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.error("unexpected GitHub download redirect")
                }
            }),
            300,
        )?
        .get(&candidate.url)
        .send()
        .await
        .map_err(|error| error.without_url().to_string())?,
    )?;
    let directory = tempfile::tempdir_in("/tmp").map_err(|error| error.to_string())?;
    let path = directory.path().join(&candidate.name);
    let mut file = std::fs::File::create_new(&path).map_err(|error| error.to_string())?;
    let mut hash = Sha256::new();
    let mut size = 0;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| error.without_url().to_string())?
    {
        size += chunk.len() as u64;
        if size > candidate.size {
            return Err("The download exceeds the GitHub package size.".into());
        }
        hash.update(&chunk);
        file.write_all(&chunk).map_err(|error| error.to_string())?;
    }
    drop(file);
    verify_download(&candidate, size, hash)?;

    tokio::task::spawn_blocking(move || {
        let _directory = directory; // Keep the private staging path alive through pacman.
        let info = stdout(pacman().args(["-Qip", "--"]).arg(&path).output())?;
        verify_package(&candidate, &info)?;
        still_installed(&candidate)?;
        installing();
        let output = Command::new(PKEXEC)
            .args([
                "--disable-internal-agent",
                PACMAN,
                "-U",
                "--noconfirm",
                "--",
            ])
            .arg(&path)
            .stdin(Stdio::null())
            .output()
            .map_err(|error| error.to_string())?;
        match output.status.code() {
            Some(126) => return Err("Authorization was canceled.".into()),
            Some(127) => {
                return Err(format!(
                    "Authorization failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ))
            }
            _ => {
                stdout(Ok(output))?;
            }
        }
        let actual = stdout(
            pacman()
                .args(["-Q", "--", &candidate.installed.name])
                .output(),
        )?;
        if actual.trim() != format!("{} {}", candidate.installed.name, candidate.version) {
            return Err("Pacman did not report the expected installed package version.".into());
        }
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed_as(name: &str) -> Installed {
        Installed {
            name: name.into(),
            version: "4.0.1-1".into(),
        }
    }

    fn release() -> Release {
        Release {
            tag_name: "v4.0.1".into(),
            draft: false,
            prerelease: false,
            assets: ["stonks", "stonks-gpui"]
                .into_iter()
                .map(|name| {
                    let name = format!("{name}-4.0.1-2-x86_64.pkg.tar.zst");
                    Asset {
                        browser_download_url: format!("{DOWNLOAD_BASE}/v4.0.1/{name}"),
                        name,
                        size: 3,
                        digest: Some(format!("sha256:{:x}", Sha256::digest(b"abc"))),
                    }
                })
                .collect(),
        }
    }

    // Arch CI supplies the real comparison tool; non-Arch Linux can still
    // exercise metadata validation without invoking the system package manager.
    #[test]
    fn selects_only_matching_identity_and_compares_pkgrel() {
        if !Path::new("/usr/bin/vercmp").exists() {
            return;
        }
        for name in ["stonks", "stonks-gpui"] {
            let candidate = select(release(), installed_as(name)).unwrap().unwrap();
            assert_eq!(candidate.name, format!("{name}-4.0.1-2-x86_64.pkg.tar.zst"));
            assert_eq!(candidate.version, "4.0.1-2");
            let mut installed = installed_as(name);
            installed.version = "1:4.0.0-1".into();
            assert!(select(release(), installed).unwrap().is_none());
        }
    }

    #[test]
    fn rejects_untrusted_release_metadata() {
        if !Path::new("/usr/bin/vercmp").exists() {
            return;
        }
        for mutate in [
            |release: &mut Release| release.assets[0].size = MAX_PACKAGE + 1,
            |release: &mut Release| release.assets[0].digest = None,
            |release: &mut Release| {
                release.assets[0].browser_download_url = "https://github.com/other/repo".into()
            },
            |release: &mut Release| release.assets[1].name = release.assets[0].name.clone(),
            |release: &mut Release| release.tag_name = "v4.0.1-rc.1".into(),
            |release: &mut Release| release.draft = true,
            |release: &mut Release| {
                release.tag_name = "v4.0.2".into();
            },
        ] {
            let mut release = release();
            mutate(&mut release);
            assert!(select(release, installed_as("stonks")).is_err());
        }
    }

    #[test]
    fn validates_size_hash_payload_and_redirect_host() {
        let asset = release().assets.remove(0);
        let candidate = Candidate {
            installed: installed_as("stonks"),
            version: "4.0.1-2".into(),
            name: asset.name,
            url: asset.browser_download_url,
            size: asset.size,
            digest: asset.digest.unwrap()[7..].into(),
        };
        let hash = |bytes| {
            let mut hash = Sha256::new();
            hash.update(bytes);
            hash
        };
        assert!(verify_download(&candidate, 3, hash(b"abc")).is_ok());
        assert!(verify_download(&candidate, 2, hash(b"abc")).is_err());
        assert!(verify_download(&candidate, 3, hash(b"xyz")).is_err());
        let info = "Name : stonks\nVersion : 4.0.1-2\nArchitecture : x86_64\n";
        assert!(verify_package(&candidate, info).is_ok());
        for (from, to) in [
            ("stonks", "stonks-gpui"),
            ("4.0.1-2", "4.0.1-1"),
            ("x86_64", "aarch64"),
        ] {
            assert!(verify_package(&candidate, &info.replace(from, to)).is_err());
        }
        for url in [
            "http://release-assets.githubusercontent.com/a",
            "https://release-assets.githubusercontent.com.evil/a",
            "https://objects.githubusercontent.com:8443/a",
        ] {
            assert!(!allowed_redirect(&url.parse().unwrap()));
        }
        assert!(allowed_redirect(
            &"https://release-assets.githubusercontent.com/a"
                .parse()
                .unwrap()
        ));
    }
}
