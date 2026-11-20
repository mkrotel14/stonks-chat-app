use serde::{Deserialize, Serialize};

pub const DEFAULT_ATTACHMENT_SIZE_LIMIT: u64 = 20_000_000;

fn default_attachment_size_limit() -> u64 {
    DEFAULT_ATTACHMENT_SIZE_LIMIT
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerEndpoints {
    pub api: String,
    pub websocket: String,
    pub autumn: Option<String>,
    pub january: Option<String>,
    pub gifbox: Option<String>,
    pub livekit_enabled: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptchaCapability {
    pub enabled: bool,
    pub site_key: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCapabilities {
    pub email_verification: bool,
    pub invite_only: bool,
    pub captcha: CaptchaCapability,
    #[serde(default = "default_attachment_size_limit")]
    pub attachment_size_limit: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerSelection {
    pub deployment_url: String,
    pub display_name: String,
    pub official: bool,
    pub api_version: String,
    #[serde(default)]
    pub capabilities: ServerCapabilities,
    pub endpoints: ServerEndpoints,
}

impl ServerSelection {
    pub fn attachment_size_limit(&self) -> u64 {
        self.capabilities.attachment_size_limit
    }

    pub fn gifbox_url(&self) -> String {
        let configured = self
            .endpoints
            .gifbox
            .as_deref()
            .map(|url| url.trim_end_matches('/'));
        let points_to_official = configured
            .is_some_and(|url| matches!(url, "https://api.gifbox.me" | "https://gifbox.me"));

        if !self.official && (configured.is_none() || points_to_official) {
            format!("{}/gifbox", self.deployment_url.trim_end_matches('/'))
        } else {
            configured.unwrap_or("https://api.gifbox.me").to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection(official: bool, gifbox: Option<&str>) -> ServerSelection {
        ServerSelection {
            deployment_url: if official {
                "https://stoat.chat"
            } else {
                "https://chat.example.com"
            }
            .into(),
            display_name: "Test".into(),
            official,
            api_version: "test".into(),
            capabilities: ServerCapabilities::default(),
            endpoints: ServerEndpoints {
                api: "https://chat.example.com/api".into(),
                websocket: "wss://chat.example.com/ws".into(),
                autumn: None,
                january: None,
                gifbox: gifbox.map(Into::into),
                livekit_enabled: false,
            },
        }
    }

    #[test]
    fn derives_local_gifbox_for_custom_deployments() {
        assert_eq!(
            selection(false, None).gifbox_url(),
            "https://chat.example.com/gifbox"
        );
    }

    #[test]
    fn migrates_the_old_official_fallback_for_custom_deployments() {
        assert_eq!(
            selection(false, Some("https://api.gifbox.me")).gifbox_url(),
            "https://chat.example.com/gifbox"
        );
    }

    #[test]
    fn preserves_advertised_and_official_gifbox_endpoints() {
        assert_eq!(
            selection(false, Some("https://media.example.com/gifs/")).gifbox_url(),
            "https://media.example.com/gifs"
        );
        assert_eq!(
            selection(true, Some("https://api.gifbox.me")).gifbox_url(),
            "https://api.gifbox.me"
        );
    }
}
