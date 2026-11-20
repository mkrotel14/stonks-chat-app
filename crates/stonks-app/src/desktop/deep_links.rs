use url::Url;

pub(super) const MAX_DEEP_LINK_BYTES: usize = 2_048;
const ID_BYTES: usize = 26;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DeepLink {
    Invite {
        code: String,
    },
    Server {
        server_id: String,
    },
    Channel {
        server_id: Option<String>,
        channel_id: String,
        message_id: Option<String>,
    },
}

impl DeepLink {
    /// Reconstruct only the validated route, never a deployment URL or invite request.
    pub(crate) fn navigation_path(&self) -> Option<String> {
        match self {
            Self::Invite { .. } => None,
            Self::Server { server_id } => Some(format!("/server/{server_id}")),
            Self::Channel {
                server_id,
                channel_id,
                message_id,
            } => {
                let mut path = server_id
                    .as_ref()
                    .map_or_else(String::new, |id| format!("/server/{id}"));
                path.push_str(&format!("/channel/{channel_id}"));
                if let Some(id) = message_id {
                    path.push('/');
                    path.push_str(id);
                }
                Some(path)
            }
        }
    }
}

pub(super) fn validate(value: &str) -> Option<DeepLink> {
    if value.is_empty()
        || value.len() > MAX_DEEP_LINK_BYTES
        || value.chars().any(char::is_control)
        || value.contains('\\')
        || contains_encoded_separator_or_control(value)
    {
        return None;
    }

    let url = Url::parse(value).ok()?;
    if url.scheme() != "stonks"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }

    let route = url.host_str()?;
    let mut segments = url.path_segments()?.collect::<Vec<_>>();
    if segments.last() == Some(&"") {
        segments.pop();
    }
    match (route, segments.as_slice()) {
        ("invite", [code]) if valid_invite(code) => Some(DeepLink::Invite {
            code: (*code).into(),
        }),
        ("server", [server_id]) if valid_id(server_id) => Some(DeepLink::Server {
            server_id: (*server_id).into(),
        }),
        ("channel", [channel_id]) => channel(None, channel_id, None),
        ("channel", [channel_id, message_id]) => channel(None, channel_id, Some(message_id)),
        ("server", [server_id, "channel", channel_id]) => {
            channel(Some(server_id), channel_id, None)
        }
        ("server", [server_id, "channel", channel_id, message_id]) => {
            channel(Some(server_id), channel_id, Some(message_id))
        }
        _ => None,
    }
}

pub(super) fn first_valid(arguments: &[std::ffi::OsString]) -> Option<(&str, DeepLink)> {
    arguments
        .iter()
        .filter_map(|arg| arg.to_str())
        .find_map(|raw| validate(raw).map(|link| (raw, link)))
}

fn channel(
    server_id: Option<&str>,
    channel_id: &str,
    message_id: Option<&str>,
) -> Option<DeepLink> {
    (server_id.is_none_or(valid_id) && valid_id(channel_id) && message_id.is_none_or(valid_id))
        .then(|| DeepLink::Channel {
            server_id: server_id.map(str::to_owned),
            channel_id: channel_id.into(),
            message_id: message_id.map(str::to_owned),
        })
}

fn valid_id(value: &str) -> bool {
    value.len() == ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte.to_ascii_uppercase(), b'A'..=b'H' | b'J'..=b'K' | b'M'..=b'N' | b'P'..=b'T' | b'V'..=b'Z'))
}

fn valid_invite(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn contains_encoded_separator_or_control(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.windows(3).any(|window| {
        if window[0] != b'%' {
            return false;
        }
        let Some(high) = hex(window[1]) else {
            return false;
        };
        let Some(low) = hex(window[2]) else {
            return false;
        };
        let decoded = high * 16 + low;
        decoded == b'/' || decoded == b'\\' || decoded.is_ascii_control()
    })
}

fn hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVER: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAT";
    const CHANNEL: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
    const MESSAGE: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAW";

    #[test]
    fn accepts_supported_routes() {
        for value in [
            "stonks://invite/hello-world",
            &format!("stonks://server/{SERVER}"),
            &format!("stonks://channel/{CHANNEL}"),
            &format!("stonks://channel/{CHANNEL}/{MESSAGE}"),
            &format!("stonks://server/{SERVER}/channel/{CHANNEL}/{MESSAGE}"),
        ] {
            assert!(validate(value).is_some(), "{value}");
        }
    }

    #[test]
    fn canonical_navigation_keeps_ids_but_never_emits_an_invite_path() {
        let path = format!("/server/{SERVER}/channel/{CHANNEL}/{MESSAGE}");
        let link = validate(&format!("stonks:/{path}")).unwrap();
        assert_eq!(link.navigation_path().as_deref(), Some(path.as_str()));
        assert_eq!(
            validate("stonks://invite/hello-world")
                .unwrap()
                .navigation_path(),
            None
        );
    }

    #[test]
    fn rejects_ambiguous_or_overprivileged_urls() {
        let oversized = format!("stonks://invite/{}", "a".repeat(MAX_DEEP_LINK_BYTES));
        for value in [
            "stonks://user@invite/code",
            "stonks://invite:44/code",
            "stonks://invite/code?next=server",
            "stonks://invite/code#fragment",
            "stonks://invite/code%2fserver",
            "stonks://invite/code%5Cserver",
            "stonks://invite/code%0a",
            "stonks://invite/co\nde",
            "stonks://invite/code/extra",
            "https://stoat.chat/invite/code",
            oversized.as_str(),
        ] {
            assert_eq!(validate(value), None, "{value}");
        }
    }

    #[test]
    fn cold_launch_selection_skips_invalid_arguments() {
        let args = [
            #[cfg(unix)]
            <std::ffi::OsString as std::os::unix::ffi::OsStringExt>::from_vec(vec![255]),
            #[cfg(windows)]
            <std::ffi::OsString as std::os::windows::ffi::OsStringExt>::from_wide(&[0xd800]),
            "stonks://invite/code?bad=1".into(),
            "--flag".into(),
            "stonks://invite/good-code".into(),
        ];
        assert_eq!(
            first_valid(&args).map(|(_, route)| route),
            Some(DeepLink::Invite {
                code: "good-code".into()
            })
        );
        assert_eq!(
            validate(&format!(
                "stonks://server/{SERVER}/channel/{CHANNEL}/{MESSAGE}"
            )),
            Some(DeepLink::Channel {
                server_id: Some(SERVER.into()),
                channel_id: CHANNEL.into(),
                message_id: Some(MESSAGE.into()),
            })
        );
    }
}
