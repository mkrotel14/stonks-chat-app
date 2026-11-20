use serde_json::{json, Value};
use url::Url;

use crate::http::{
    AuthenticatedContext, HttpPolicy, HttpPolicyError, RequestClass, MAX_JSON_RESPONSE_BYTES,
};

pub(super) struct VoiceCredentials {
    pub(super) url: String,
    pub(super) token: String,
}

fn policy_error(error: HttpPolicyError) -> &'static str {
    match error {
        HttpPolicyError::Unavailable => "the request was unavailable",
        HttpPolicyError::Timeout => "the request timed out",
        HttpPolicyError::InvalidResponse => "the response was invalid",
        HttpPolicyError::BodyTooLarge => "the response body was too large",
        HttpPolicyError::Cancelled => "the request was cancelled",
    }
}

pub(super) fn voice_url(api: &str, channel_id: &str) -> Result<Url, String> {
    let mut url = Url::parse(api).map_err(|_| "The API address is invalid.")?;
    url.path_segments_mut()
        .map_err(|_| "The API address cannot be extended.".to_string())?
        .pop_if_empty()
        .extend(["channels", channel_id, "join_call"]);
    Ok(url)
}

pub(super) async fn fetch_credentials(
    policy: &HttpPolicy,
    context: &AuthenticatedContext,
    channel_id: &str,
) -> Result<VoiceCredentials, String> {
    let configuration_request = policy.client().get(&context.server.endpoints.api);
    let configuration_response = policy
        .execute_authenticated(
            context,
            configuration_request,
            RequestClass::Standard,
            MAX_JSON_RESPONSE_BYTES,
        )
        .await
        .map_err(|error| {
            format!(
                "Could not load the voice node list: {}",
                policy_error(error)
            )
        })?;
    let configuration = configuration_response.consume(
        |status, _, bytes| -> Result<Value, String> {
            if !status.is_success() {
                return Err(format!(
                    "Could not load the voice node list (HTTP {}).",
                    status.as_u16()
                ));
            }
            serde_json::from_slice::<Value>(bytes)
                .map_err(|error| format!("The voice node list was invalid: {error}"))
        },
        || {
            format!(
                "Could not load the voice node list: {}",
                policy_error(HttpPolicyError::Cancelled)
            )
        },
    )?;
    let node = voice_node(&configuration)
        .ok_or("This Stoat deployment did not advertise an available voice node.")?;
    let advertised_url = voice_public_url(&configuration, node).map(ToOwned::to_owned);
    let request = policy
        .client()
        .post(voice_url(&context.server.endpoints.api, channel_id.trim())?)
        .header("x-session-token", &context.token)
        .json(&json!({ "node": node, "force_disconnect": true }));
    let response = policy
        .execute_authenticated(
            context,
            request,
            RequestClass::Standard,
            MAX_JSON_RESPONSE_BYTES,
        )
        .await
        .map_err(|error| format!("Could not join the voice channel: {}", policy_error(error)))?;
    let value = response.consume(
        |status, _, bytes| -> Result<Value, String> {
            if !status.is_success() {
                return Err(format!(
                    "Could not join the voice channel (HTTP {}).",
                    status.as_u16()
                ));
            }
            serde_json::from_slice(bytes).map_err(|_| "The voice join response was invalid.".into())
        },
        || {
            format!(
                "Could not join the voice channel: {}",
                policy_error(HttpPolicyError::Cancelled)
            )
        },
    )?;
    // Self-hosted join responses can contain an internal Docker address.
    let url = advertised_url
        .as_deref()
        .or_else(|| value.get("url").and_then(Value::as_str))
        .ok_or("The voice server did not return a LiveKit URL.")?
        .to_owned();
    validate_voice_endpoint(&context.server.endpoints.api, &url)?;

    Ok(VoiceCredentials {
        url,
        token: value
            .get("token")
            .and_then(Value::as_str)
            .filter(|token| !token.trim().is_empty())
            .ok_or("The voice server did not return a LiveKit token.")?
            .to_owned(),
    })
}

pub(super) fn voice_node(configuration: &Value) -> Option<&str> {
    configuration
        .get("features")?
        .get("livekit")?
        .get("nodes")?
        .as_array()?
        .iter()
        .find_map(|node| node.get("name").and_then(Value::as_str))
}

pub(super) fn voice_public_url<'a>(configuration: &'a Value, node_name: &str) -> Option<&'a str> {
    configuration
        .get("features")?
        .get("livekit")?
        .get("nodes")?
        .as_array()?
        .iter()
        .find(|node| node.get("name").and_then(Value::as_str) == Some(node_name))?
        .get("public_url")?
        .as_str()
}

fn url_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(host)) => {
            host.eq_ignore_ascii_case("localhost")
                || host.to_ascii_lowercase().ends_with(".localhost")
        }
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    }
}

#[cfg(test)]
pub(super) fn loopback_host(value: &str) -> bool {
    Url::parse(value).is_ok_and(|url| url_loopback(&url))
}

fn validate_voice_endpoint(api: &str, endpoint: &str) -> Result<(), String> {
    let api = Url::parse(api).map_err(|_| "The saved API address is invalid.")?;
    let endpoint =
        Url::parse(endpoint).map_err(|_| "The server returned an invalid LiveKit address.")?;
    let clean = endpoint.host_str().is_some()
        && endpoint.username().is_empty()
        && endpoint.password().is_none()
        && endpoint.query().is_none()
        && endpoint.fragment().is_none();
    let local_development = api.scheme() == "http" && url_loopback(&api);
    let secure_transport = endpoint.scheme() == "wss";
    let local_transport = endpoint.scheme() == "ws" && url_loopback(&endpoint);

    if !clean
        || !(secure_transport || local_development && local_transport)
        || (url_loopback(&endpoint) && !url_loopback(&api))
    {
        return Err(
            "The server returned an invalid or insecure LiveKit address. Configure a secure public voice node URL."
                .into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn creates_join_call_url() {
        assert_eq!(
            voice_url("https://api.example.com", "01VOICE")
                .unwrap()
                .as_str(),
            "https://api.example.com/channels/01VOICE/join_call"
        );
    }

    #[test]
    fn selects_advertised_voice_node() {
        let configuration = json!({
            "features": { "livekit": { "enabled": true, "nodes": [
                { "name": "sa-east", "public_url": "wss://voice.example.com" }
            ] } }
        });
        assert_eq!(voice_node(&configuration), Some("sa-east"));
        assert_eq!(
            voice_public_url(&configuration, "sa-east"),
            Some("wss://voice.example.com")
        );
    }

    #[test]
    fn detects_internal_livekit_urls() {
        assert!(loopback_host("ws://localhost:7880"));
        assert!(loopback_host("ws://dev.localhost:7880"));
        assert!(loopback_host("wss://127.0.0.42/livekit"));
        assert!(loopback_host("ws://[::1]:7880"));
        assert!(!loopback_host("wss://chat.example.com/livekit"));
    }

    #[test]
    fn enforces_secure_livekit_transport_with_a_loopback_development_exception() {
        assert!(validate_voice_endpoint(
            "https://api.example.com",
            "wss://voice.example.net/livekit"
        )
        .is_ok());
        assert!(
            validate_voice_endpoint("http://localhost:14702/api", "ws://127.0.0.42:7880").is_ok()
        );

        for endpoint in [
            "ws://voice.example.net/livekit",
            "https://voice.example.net/livekit",
            "wss://person:secret@voice.example.net/livekit",
            "wss://voice.example.net/livekit?token=secret",
            "wss://voice.example.net/livekit#fragment",
            "wss://localhost:7880/livekit",
        ] {
            assert!(validate_voice_endpoint("https://api.example.com", endpoint).is_err());
        }
        assert!(validate_voice_endpoint(
            "http://localhost:14702/api",
            "ws://192.168.1.20:7880/livekit"
        )
        .is_err());
    }
}
