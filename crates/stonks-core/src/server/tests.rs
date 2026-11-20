use super::*;
use super::probe::*;
use crate::error::CommandError;
use url::Url;
use reqwest::StatusCode;

fn error_value(error: CommandError) -> serde_json::Value {
    serde_json::to_value(error).unwrap()
}

fn selection(
    deployment: &str,
    api: &str,
    websocket: &str,
    autumn: Option<&str>,
    january: Option<&str>,
    gifbox: Option<&str>,
) -> ServerSelection {
    ServerSelection {
        deployment_url: deployment.into(),
        display_name: "Test".into(),
        official: false,
        api_version: "0.16.1".into(),
        capabilities: ServerCapabilities::default(),
        endpoints: ServerEndpoints {
            api: api.into(),
            websocket: websocket.into(),
            autumn: autumn.map(Into::into),
            january: january.map(Into::into),
            gifbox: gifbox.map(Into::into),
            livekit_enabled: false,
        },
    }
}

#[test]
fn derives_self_hosted_api_from_web_url() {
    let input = normalize_input("https://chat.example.com/app").unwrap();
    let candidates = api_candidates(&input).unwrap();
    assert_eq!(candidates[0].as_str(), "https://chat.example.com/api");
}

#[test]
fn preserves_explicit_api_url_and_custom_port() {
    let input = normalize_input("https://example.com:8443/api").unwrap();
    let candidates = api_candidates(&input).unwrap();
    assert_eq!(candidates[0].as_str(), "https://example.com:8443/api");
}

#[test]
fn maps_only_canonical_official_https_hosts_to_official_api() {
    for value in ["https://stoat.chat/app", "https://api.stoat.chat"] {
        let input = normalize_input(value).unwrap();
        let candidates = api_candidates(&input).unwrap();
        assert_eq!(candidates[0].as_str(), format!("{OFFICIAL_API}/"));
    }

    let custom = Url::parse("https://stoat.chat:8443/app").unwrap();
    assert_eq!(
        api_candidates(&custom).unwrap()[0].as_str(),
        "https://stoat.chat:8443/api"
    );
    let deceptive = Url::parse("https://stoat.chat.example/app").unwrap();
    assert_eq!(
        api_candidates(&deceptive).unwrap()[0].as_str(),
        "https://stoat.chat.example/api"
    );
}

#[test]
fn probe_statuses_serialize_as_stable_safe_errors() {
    for (status, code) in [
        (StatusCode::BAD_REQUEST, "invalid_input"),
        (StatusCode::NOT_FOUND, "not_found"),
        (StatusCode::TOO_MANY_REQUESTS, "unavailable"),
        (StatusCode::INTERNAL_SERVER_ERROR, "unavailable"),
    ] {
        assert_eq!(
            error_value(probe_status_error(status)),
            serde_json::json!({
                "code": code,
                "message": "The Stoat deployment could not be verified."
            })
        );
    }
}

#[test]
fn deployment_input_accepts_secure_and_loopback_addresses() {
    for value in [
        "https://chat.example.com",
        "https://chat.example.com:8443/api",
        "http://localhost:14702",
        "http://dev.localhost:14702",
        "http://127.0.0.42:14702",
        "http://[::1]:14702",
    ] {
        assert!(normalize_input(value).is_ok(), "{value}");
    }
}

#[test]
fn deployment_input_rejects_plaintext_non_loopback_and_ambiguous_urls() {
    for value in [
        "http://chat.example.com",
        "http://192.168.1.20:8000",
        "http://10.0.0.5",
        "http://172.16.0.5",
        "http://chat.local",
        "http://attackerlocalhost",
        "file:///private/server.json",
        "https://person:secret@example.com",
        "https://example.com/api?token=secret",
        "https://example.com/api#fragment",
        "not a URL",
    ] {
        assert!(normalize_input(value).is_err(), "{value}");
    }
}

#[test]
fn official_and_secure_self_hosted_selections_are_valid() {
    let official = selection(
        "https://stoat.chat",
        "https://api.stoat.chat",
        "wss://events.stoat.chat",
        Some("https://cdn.stoatusercontent.com"),
        Some("https://proxy.stoatusercontent.com"),
        Some("https://api.gifbox.me"),
    );
    assert!(validate_selection(&official).is_ok());

    let self_hosted = selection(
        "https://chat.example.com:8443",
        "https://chat.example.com:8443/api",
        "wss://chat.example.com:8443/ws",
        Some("https://chat.example.com:8443/autumn"),
        Some("https://media.example.net/january"),
        Some("https://chat.example.com:8443/gifbox"),
    );
    assert!(validate_selection(&self_hosted).is_ok());
}

#[test]
fn loopback_plaintext_requires_every_plaintext_endpoint_to_be_loopback() {
    let loopback = selection(
        "http://localhost:14702",
        "http://127.0.0.1:14702/api",
        "ws://[::1]:14702/ws",
        Some("http://localhost:14702/autumn"),
        None,
        Some("http://dev.localhost:14702/gifbox"),
    );
    assert!(validate_selection(&loopback).is_ok());

    for invalid in [
        selection(
            "https://chat.example.com",
            "https://chat.example.com/api",
            "ws://localhost:14702/ws",
            None,
            None,
            None,
        ),
        selection(
            "https://chat.example.com",
            "https://chat.example.com/api",
            "wss://localhost:14702/ws",
            None,
            None,
            None,
        ),
        selection(
            "https://chat.example.com",
            "https://localhost:14702/api",
            "wss://chat.example.com/ws",
            None,
            None,
            None,
        ),
        selection(
            "https://chat.example.com",
            "https://chat.example.com/api",
            "wss://chat.example.com/ws",
            Some("https://127.0.0.1:14702/autumn"),
            None,
            None,
        ),
        selection(
            "http://localhost:14702",
            "http://192.168.1.20:14702/api",
            "ws://localhost:14702/ws",
            None,
            None,
            None,
        ),
        selection(
            "http://localhost:14702",
            "http://localhost:14702/api",
            "ws://chat.example.com/ws",
            None,
            None,
            None,
        ),
    ] {
        assert!(validate_selection(&invalid).is_err());
    }
}

#[test]
fn selections_reject_credentials_queries_fragments_and_wrong_schemes() {
    for invalid in [
        selection(
            "https://chat.example.com",
            "https://person:secret@chat.example.com/api",
            "wss://chat.example.com/ws",
            None,
            None,
            None,
        ),
        selection(
            "https://chat.example.com",
            "https://chat.example.com/api?token=secret",
            "wss://chat.example.com/ws",
            None,
            None,
            None,
        ),
        selection(
            "https://chat.example.com",
            "https://chat.example.com/api",
            "https://chat.example.com/ws",
            None,
            None,
            None,
        ),
        selection(
            "https://chat.example.com",
            "https://chat.example.com/api",
            "wss://chat.example.com/ws#fragment",
            Some("ftp://chat.example.com/autumn"),
            None,
            None,
        ),
    ] {
        assert!(validate_selection(&invalid).is_err());
    }
}

#[test]
fn relative_service_references_preserve_origin_and_transport_security() {
    let secure_api = Url::parse("https://chat.example.com:8443/api").unwrap();
    assert_eq!(
        resolve_http_reference(&secure_api, "/autumn")
            .unwrap()
            .as_str(),
        "https://chat.example.com:8443/autumn"
    );
    assert_eq!(
        resolve_websocket_reference(&secure_api, "/ws")
            .unwrap()
            .as_str(),
        "wss://chat.example.com:8443/ws"
    );

    let local_api = Url::parse("http://localhost:14702/api").unwrap();
    assert_eq!(
        resolve_websocket_reference(&local_api, "/ws")
            .unwrap()
            .as_str(),
        "ws://localhost:14702/ws"
    );
    assert!(resolve_http_reference(&secure_api, "//attacker.example/autumn").is_err());
    assert!(resolve_websocket_reference(&secure_api, "https://chat.example.com/ws").is_err());
    assert!(resolve_websocket_reference(&secure_api, "ws://chat.example.com/ws").is_err());
}

#[test]
fn enabled_services_require_valid_endpoints() {
    let api = Url::parse("https://chat.example.com/api").unwrap();
    let features = serde_json::json!({
        "autumn": { "enabled": true },
        "january": { "enabled": false },
        "gifbox": { "url": "/gifbox" }
    });
    assert!(feature_service_url(&features, "autumn", &api).is_err());
    assert_eq!(
        feature_service_url(&features, "january", &api).unwrap(),
        None
    );
    assert_eq!(
        feature_service_url(&features, "gifbox", &api).unwrap(),
        Some("https://chat.example.com/gifbox".into())
    );
}

#[test]
fn parses_the_live_official_node_configuration_shape() {
    let input = Url::parse("https://stoat.chat").unwrap();
    let api = Url::parse("https://api.stoat.chat/").unwrap();
    let config = serde_json::json!({
        "stoat": "0.16.1",
        "revolt": "0.16.1",
        "ws": "wss://events.stoat.chat",
        "features": {
            "autumn": { "url": "https://cdn.stoatusercontent.com" },
            "january": { "url": "https://proxy.stoatusercontent.com" },
            "livekit": {
                "enabled": true,
                "nodes": [{ "public_url": "wss://livekit.hel1.voice.stoat.chat" }]
            }
        },
        "limits": {
            "new_user": { "file_upload_size_limits": { "attachments": 20000000 } },
            "default": { "file_upload_size_limits": { "attachments": 25000000 } }
        }
    });

    let selected = selection_from_config(&input, api, &config).unwrap();
    assert_eq!(selected.deployment_url, "https://stoat.chat");
    assert_eq!(selected.endpoints.api, "https://api.stoat.chat");
    assert_eq!(selected.endpoints.websocket, "wss://events.stoat.chat/");
    assert_eq!(
        selected.endpoints.autumn.as_deref(),
        Some("https://cdn.stoatusercontent.com/")
    );
    assert_eq!(
        selected.endpoints.january.as_deref(),
        Some("https://proxy.stoatusercontent.com/")
    );
    assert_eq!(selected.api_version, "0.16.1");
    assert_eq!(selected.capabilities.attachment_size_limit, 20_000_000);
    assert!(selected.official);
    assert!(selected.endpoints.livekit_enabled);
}

#[test]
fn parses_stock_self_hosted_and_loopback_relative_endpoints() {
    let config = serde_json::json!({
        "stoat": "0.16.1",
        "revolt": "0.16.1",
        "ws": "/ws",
        "features": {
            "autumn": { "enabled": true, "url": "/autumn" },
            "january": { "enabled": true, "url": "/january" }
        }
    });
    let selected = selection_from_config(
        &Url::parse("https://chat.example.com").unwrap(),
        Url::parse("https://chat.example.com/api").unwrap(),
        &config,
    )
    .unwrap();
    assert_eq!(selected.endpoints.websocket, "wss://chat.example.com/ws");
    assert_eq!(
        selected.endpoints.autumn.as_deref(),
        Some("https://chat.example.com/autumn")
    );
    assert_eq!(
        selected.endpoints.january.as_deref(),
        Some("https://chat.example.com/january")
    );

    let local = selection_from_config(
        &Url::parse("http://localhost:14702").unwrap(),
        Url::parse("http://localhost:14702/api").unwrap(),
        &config,
    )
    .unwrap();
    assert_eq!(local.endpoints.websocket, "ws://localhost:14702/ws");
    assert_eq!(
        local.endpoints.autumn.as_deref(),
        Some("http://localhost:14702/autumn")
    );
}

#[test]
fn node_configuration_rejects_missing_or_insecure_advertisements() {
    let input = Url::parse("https://chat.example.com").unwrap();
    let api = Url::parse("https://chat.example.com/api").unwrap();
    for config in [
        serde_json::json!({ "stoat": "0.16.1" }),
        serde_json::json!({ "stoat": "0.16.1", "ws": "" }),
        serde_json::json!({ "stoat": "0.16.1", "ws": "https://chat.example.com/ws" }),
        serde_json::json!({ "stoat": "0.16.1", "ws": "ws://chat.example.com/ws" }),
        serde_json::json!({
            "stoat": "0.16.1",
            "ws": "wss://chat.example.com/ws",
            "features": { "autumn": { "enabled": true } }
        }),
        serde_json::json!({ "stoat": "0.16.1", "ws": "//attacker.example/ws" }),
        serde_json::json!({
            "stoat": "0.16.1",
            "ws": "wss://chat.example.com/ws",
            "features": { "autumn": { "url": "http://192.168.1.20/autumn" } }
        }),
    ] {
        assert!(selection_from_config(&input, api.clone(), &config).is_err());
    }
}

#[test]
fn parses_only_safe_well_known_api_values() {
    let base = Url::parse("https://chat.example.com:8443/.well-known/stoat").unwrap();
    assert_eq!(
        well_known_api_value(&base, &serde_json::json!({ "api": "/api" }))
            .unwrap()
            .as_str(),
        "https://chat.example.com:8443/api"
    );
    assert_eq!(
        well_known_api_value(
            &base,
            &serde_json::json!({ "api": "https://api.example.net:9443" })
        )
        .unwrap()
        .as_str(),
        "https://api.example.net:9443/"
    );
    for value in [
        serde_json::json!({}),
        serde_json::json!({ "api": "//attacker.example/api" }),
        serde_json::json!({ "api": "https://person:secret@example.com/api" }),
        serde_json::json!({ "api": "http://chat.example.com/api" }),
    ] {
        assert!(well_known_api_value(&base, &value).is_err());
    }
}
