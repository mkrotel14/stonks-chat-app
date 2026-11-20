//! Read-only discovery of the public GPUI release. Asset metadata is ignored.

use semver::Version;
use serde::Deserialize;
use std::time::Duration;

const RELEASE_URL: &str = "https://api.github.com/repos/mkrotel14/stonks-chat-app/releases/latest";
const MAX_METADATA_BYTES: usize = 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum DiscoveryError {
    Unavailable,
    InvalidMetadata,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

fn stable_version(metadata: &[u8]) -> Result<Version, DiscoveryError> {
    let release: Release =
        serde_json::from_slice(metadata).map_err(|_| DiscoveryError::InvalidMetadata)?;
    let tag = &release.tag_name;
    let version = Version::parse(tag.strip_prefix(['v', 'V']).unwrap_or(tag))
        .map_err(|_| DiscoveryError::InvalidMetadata)?;
    if release.draft || release.prerelease || !version.pre.is_empty() {
        return Err(DiscoveryError::InvalidMetadata);
    }
    Ok(version)
}

/// Fetch release metadata without using the authenticated chat HTTP client.
pub async fn latest_public_version() -> Result<Version, DiscoveryError> {
    use DiscoveryError::*;

    let client = reqwest::Client::builder()
        .user_agent("Stonks-GPUI")
        .redirect(reqwest::redirect::Policy::none())
        .https_only(true)
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| Unavailable)?;
    let mut response = client
        .get(RELEASE_URL)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|_| Unavailable)?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(Unavailable);
    }
    let mut metadata = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Unavailable)? {
        if chunk.len() > MAX_METADATA_BYTES - metadata.len() {
            return Err(InvalidMetadata);
        }
        metadata.extend_from_slice(&chunk);
    }
    stable_version(&metadata)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_version_and_ignores_even_plausible_gpui_assets() {
        let metadata = br#"{"tag_name":"v1.20.3+build.4","draft":false,"prerelease":false,
            "assets":[{"name":"stonks-gpui-1.20.3.x86_64.rpm","browser_download_url":"https://example.com"}]}"#;
        assert_eq!(
            stable_version(metadata).unwrap().to_string(),
            "1.20.3+build.4"
        );
    }

    #[tokio::test]
    #[ignore = "uses the live GitHub API"]
    async fn discovers_live_public_release() {
        let version = latest_public_version().await.unwrap();
        assert!(version.pre.is_empty());
        eprintln!("Latest public release: {version}");
    }

    #[test]
    fn rejects_malformed_or_nonstable_metadata() {
        for (tag, draft, prerelease) in [
            ("vv1.2.3", false, false),
            ("v1.2.3.4", false, false),
            ("v1.02.3", false, false),
            ("v1.2.3-rc.1", false, false),
            ("v1.2.3", true, false),
            ("v1.2.3", false, true),
        ] {
            let json = serde_json::json!({"tag_name":tag,"draft":draft,"prerelease":prerelease});
            assert_eq!(
                stable_version(json.to_string().as_bytes()),
                Err(DiscoveryError::InvalidMetadata)
            );
        }
        assert_eq!(stable_version(b"{}"), Err(DiscoveryError::InvalidMetadata));
    }
}
