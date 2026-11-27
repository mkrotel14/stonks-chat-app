//! Verified managed updates from the standalone GPUI release repository.

use reqwest::{redirect::Policy, Client, Response};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

#[cfg(target_os = "linux")]
#[path = "managed/arch.rs"]
mod arch;
#[cfg(target_os = "linux")]
#[path = "managed/linux.rs"]
mod linux;
#[cfg(target_os = "linux")]
#[path = "managed/rpm.rs"]
mod rpm;
#[cfg(target_os = "windows")]
#[path = "managed/windows.rs"]
mod windows;

#[cfg(target_os = "linux")]
const MANAGED: &str = "/usr/bin/stonks-app";
const RELEASE_API: &str = "https://api.github.com/repos/mkrotel14/stonks-chat-app/releases/latest";
const DOWNLOAD_BASE: &str = "https://github.com/mkrotel14/stonks-chat-app/releases/download";
const MAX_PACKAGE: u64 = 512 * 1024 * 1024;
type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    Arch,
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    Rpm,
    #[cfg_attr(all(target_os = "linux", not(test)), allow(dead_code))]
    Windows,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Installed {
    kind: Kind,
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
    pub(super) fn kind(&self) -> Kind {
        self.installed.kind
    }

    pub(crate) fn label(&self) -> String {
        format!(
            "{} {} → {}",
            self.installed.name, self.installed.version, self.version
        )
    }
}

fn numeric(part: &str) -> bool {
    !part.is_empty() && part.bytes().all(|c| c.is_ascii_digit())
}

fn version_parts(version: &str) -> Option<[u64; 3]> {
    let parts: Vec<_> = version.split('.').collect();
    let [major, minor, patch] = parts.as_slice() else {
        return None;
    };
    if !parts.iter().all(|part| numeric(part)) {
        return None;
    }
    Some([
        major.parse().ok()?,
        minor.parse().ok()?,
        patch.parse().ok()?,
    ])
}

fn installed(kind: Kind) -> Result<Installed> {
    match kind {
        #[cfg(target_os = "linux")]
        Kind::Arch => arch::installed(),
        #[cfg(target_os = "linux")]
        Kind::Rpm => rpm::installed(),
        #[cfg(target_os = "windows")]
        Kind::Windows => windows::installed(),
        _ => Err("Unsupported update platform.".into()),
    }
}

pub(super) fn managed_kind() -> Option<Kind> {
    #[cfg(target_os = "linux")]
    {
        if arch::eligible() {
            return Some(Kind::Arch);
        }
        rpm::installed().ok().map(|_| Kind::Rpm)
    }
    #[cfg(target_os = "windows")]
    windows::installed().ok().map(|_| Kind::Windows)
}

fn newer(kind: Kind, candidate: &str, installed: &str) -> Result<bool> {
    match kind {
        #[cfg(target_os = "linux")]
        Kind::Arch => arch::newer(candidate, installed),
        Kind::Rpm => {
            let candidate = rpm_version(candidate).ok_or("Invalid Fedora update version.")?;
            let installed = rpm_version(installed).ok_or("Invalid installed Fedora version.")?;
            Ok(candidate > installed)
        }
        Kind::Windows => {
            let candidate = version_parts(candidate).ok_or("Invalid Windows update version.")?;
            let installed = version_parts(installed).ok_or("Invalid installed Windows version.")?;
            Ok(candidate > installed)
        }
        #[cfg(not(target_os = "linux"))]
        Kind::Arch => Err("Arch version comparison is unavailable.".into()),
    }
}

fn rpm_version(value: &str) -> Option<([u64; 3], u64)> {
    let (version, release) = value.split_once('-')?;
    let revision = release.strip_suffix(".fc44")?;
    if !numeric(revision) {
        return None;
    }
    Some((version_parts(version)?, revision.parse().ok()?))
}

fn select(release: Release, installed: Installed) -> Result<Option<Candidate>> {
    let tag = &release.tag_name;
    let version = tag.strip_prefix(['v', 'V']).unwrap_or(tag);
    if release.draft || release.prerelease || version_parts(version).is_none() {
        return Err("Invalid stable GPUI release tag.".into());
    }
    let prefix = format!("{}-{version}-", installed.name);
    let windows_name = format!("Stonks.GPUI_{version}_x64-setup.exe");
    let mut matches = release.assets.iter().filter_map(|asset| {
        let package_version = match installed.kind {
            Kind::Arch => {
                let revision = asset
                    .name
                    .strip_prefix(&prefix)?
                    .strip_suffix("-x86_64.pkg.tar.zst")?;
                if !numeric(revision) {
                    return None;
                }
                format!("{version}-{revision}")
            }
            Kind::Rpm => {
                let revision = asset
                    .name
                    .strip_prefix(&prefix)?
                    .strip_suffix(".fc44.x86_64.rpm")?;
                if !numeric(revision) {
                    return None;
                }
                format!("{version}-{revision}.fc44")
            }
            Kind::Windows if asset.name == windows_name => version.to_owned(),
            Kind::Windows => return None,
        };
        Some((asset, package_version))
    });
    let Some((asset, package_version)) = matches.next() else {
        let baseline = if installed.kind == Kind::Rpm {
            format!("{version}-0.fc44")
        } else {
            version.into()
        };
        if !newer(installed.kind, &baseline, &installed.version)? {
            return Ok(None);
        }
        return Err("The latest GPUI release has no matching update package.".into());
    };
    if matches.next().is_some() {
        return Err("The GPUI release contains ambiguous update packages.".into());
    }
    if !newer(installed.kind, &package_version, &installed.version)? {
        return Ok(None);
    }
    let url = format!("{DOWNLOAD_BASE}/{tag}/{}", asset.name);
    let digest = asset
        .digest
        .as_deref()
        .and_then(|value| value.strip_prefix("sha256:"))
        .filter(|hex| hex.len() == 64 && hex.bytes().all(|c| c.is_ascii_hexdigit()))
        .ok_or("The update package has no valid GitHub SHA-256 digest.")?;
    if asset.browser_download_url != url || !(1..=MAX_PACKAGE).contains(&asset.size) {
        return Err("Invalid update package URL or size in GitHub metadata.".into());
    }
    Ok(Some(Candidate {
        installed,
        version: package_version,
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

pub(super) async fn check(kind: Kind) -> Result<Option<Candidate>> {
    let installed = tokio::task::spawn_blocking(move || installed(kind))
        .await
        .map_err(|error| error.to_string())??;
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

fn still_installed(candidate: &Candidate) -> Result<()> {
    if installed(candidate.kind())? != candidate.installed {
        return Err("The installed package changed. Check for updates again.".into());
    }
    Ok(())
}

async fn download(candidate: &Candidate, staging: &Path) -> Result<(tempfile::TempDir, PathBuf)> {
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
    let directory = tempfile::Builder::new()
        .prefix("stonks-update-")
        .tempdir_in(staging)
        .map_err(|error| error.to_string())?;
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
    verify_download(candidate, size, hash)?;
    Ok((directory, path))
}

pub(super) enum InstalledAction {
    #[cfg(target_os = "linux")]
    ManualRestart,
    #[cfg(target_os = "linux")]
    Restart,
    #[cfg(target_os = "windows")]
    Quit,
}

pub(super) async fn install(
    candidate: Candidate,
    installing: impl FnOnce() + Send + 'static,
) -> Result<InstalledAction> {
    let kind = candidate.kind();
    let preflight = candidate.clone();
    let staging = tokio::task::spawn_blocking(move || {
        still_installed(&preflight)?;
        #[cfg(target_os = "linux")]
        {
            linux::preflight(kind)?;
            Ok::<_, String>(PathBuf::from("/tmp"))
        }
        #[cfg(target_os = "windows")]
        windows::install_dir()
    })
    .await
    .map_err(|error| error.to_string())??;
    let (directory, path) = download(&candidate, &staging).await?;
    tokio::task::spawn_blocking(move || {
        #[cfg(target_os = "linux")]
        {
            let _directory = directory; // Keep private staging alive through authorization and install.
            match kind {
                Kind::Arch => {
                    arch::install(&candidate, &path, installing)?;
                    Ok(InstalledAction::ManualRestart)
                }
                Kind::Rpm => {
                    rpm::install(&candidate, &path, installing)?;
                    Ok(InstalledAction::Restart)
                }
                Kind::Windows => Err("Unsupported update platform.".into()),
            }
        }
        #[cfg(target_os = "windows")]
        {
            still_installed(&candidate)?;
            if windows::install_dir()? != staging {
                return Err("The installation directory changed. Check for updates again.".into());
            }
            installing();
            windows::launch(directory, &path, &candidate)?;
            Ok(InstalledAction::Quit)
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed_as(name: &str) -> Installed {
        Installed {
            kind: Kind::Arch,
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

    #[test]
    fn selects_only_current_gpui_fedora_and_windows_assets() {
        for (kind, package, installed_version, filename, version, decoys) in [
            (
                Kind::Rpm,
                "stonks-gpui",
                "4.0.1-1.fc44",
                "stonks-gpui-4.0.1-2.fc44.x86_64.rpm",
                "4.0.1-2.fc44",
                [
                    "Stonks.Chat-4.0.1-2.x86_64.rpm",
                    "stonks-gpui-4.0.1-2.fc43.x86_64.rpm",
                    "stonks-gpui-4.0.1-2.fc44.aarch64.rpm",
                ],
            ),
            (
                Kind::Windows,
                "Stonks GPUI",
                "4.0.0",
                "Stonks.GPUI_4.0.1_x64-setup.exe",
                "4.0.1",
                [
                    "Stonks.Chat_4.0.1_x64-setup.exe",
                    "Stonks.GPUI_4.0.1_arm64-setup.exe",
                    "Stonks.GPUI_4.0.2_x64-setup.exe",
                ],
            ),
        ] {
            let installed = Installed {
                kind,
                name: package.into(),
                version: installed_version.into(),
            };
            let asset = |name: &str| Asset {
                name: name.into(),
                browser_download_url: format!("{DOWNLOAD_BASE}/v4.0.1/{name}"),
                size: 3,
                digest: Some(format!("sha256:{:x}", Sha256::digest(b"abc"))),
            };
            let make_release = || {
                let mut release = release();
                release.assets.extend(decoys.map(asset));
                release.assets.push(asset(filename));
                release
            };
            let selected = select(make_release(), installed.clone()).unwrap().unwrap();
            assert_eq!(selected.name, filename);
            assert_eq!(selected.version, version);
            let mut current = installed.clone();
            current.version = version.into();
            assert!(select(make_release(), current).unwrap().is_none());
            let mut duplicate = make_release();
            duplicate.assets.push(asset(filename));
            assert!(select(duplicate, installed.clone()).is_err());
            let mut missing_digest = make_release();
            missing_digest.assets.last_mut().unwrap().digest = None;
            assert!(select(missing_digest, installed.clone()).is_err());
            let mut bad_url = make_release();
            bad_url
                .assets
                .last_mut()
                .unwrap()
                .browser_download_url
                .push_str("?redirect=1");
            assert!(select(bad_url, installed.clone()).is_err());
            let mut legacy_only = make_release();
            legacy_only.assets.pop();
            assert!(!matches!(select(legacy_only, installed), Ok(Some(_))));
        }
    }

    // Arch CI supplies the real comparison tool; non-Arch Linux can still
    // exercise metadata validation without invoking the system package manager.
    #[cfg(target_os = "linux")]
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

    #[cfg(target_os = "linux")]
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
        #[cfg(target_os = "linux")]
        let info = "Name : stonks\nVersion : 4.0.1-2\nArchitecture : x86_64\n";
        #[cfg(target_os = "linux")]
        assert!(arch::verify_package(&candidate, info).is_ok());
        #[cfg(target_os = "linux")]
        for (from, to) in [
            ("stonks", "stonks-gpui"),
            ("4.0.1-2", "4.0.1-1"),
            ("x86_64", "aarch64"),
        ] {
            assert!(arch::verify_package(&candidate, &info.replace(from, to)).is_err());
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
