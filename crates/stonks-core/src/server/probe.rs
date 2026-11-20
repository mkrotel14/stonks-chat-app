//! Deployment probing: turns a user-entered deployment URL into a validated
//! `ServerSelection` using node configuration, `.well-known/stoat` discovery,
//! and endpoint validation.

use std::time::Duration;

use futures_util::StreamExt;
use reqwest::{Client, StatusCode};
use serde_json::Value;
use url::Url;

use crate::error::{CommandError, ErrorCode};

use super::endpoints::{
    deployment_origin, normalize_input, secure_http_endpoint, secure_websocket_endpoint,
    validate_selection,
};
use super::models::{
    CaptchaCapability, ServerCapabilities, ServerEndpoints, ServerSelection,
    DEFAULT_ATTACHMENT_SIZE_LIMIT,
};
use super::{OFFICIAL_API, OFFICIAL_DEPLOYMENT};

const MAX_DISCOVERY_BYTES: u64 = 1024 * 1024;
const MAX_WELL_KNOWN_BYTES: u64 = 64 * 1024;
const MAX_PROBE_REDIRECTS: usize = 3;

const fn server_error(code: ErrorCode, message: &'static str) -> CommandError {
    CommandError::new(code, message)
}

pub(crate) fn api_candidates(input: &Url) -> Result<Vec<Url>, CommandError> {
    let official_host = matches!(input.host_str(), Some("stoat.chat" | "api.stoat.chat"));
    if input.scheme() == "https" && input.port_or_known_default() == Some(443) && official_host {
        return Ok(vec![
            Url::parse(OFFICIAL_API).expect("official API URL is valid")
        ]);
    }

    let origin = deployment_origin(input)?;
    let path = input.path().trim_end_matches('/');
    let mut candidates = Vec::new();
    if path.ends_with("/api")
        || input
            .host_str()
            .is_some_and(|host| host.starts_with("api."))
    {
        candidates.push(input.clone());
    }
    candidates.push(origin.join("api").map_err(|_| {
        server_error(
            ErrorCode::InvalidInput,
            "The deployment address has no usable API endpoint.",
        )
    })?);
    if !path.is_empty() && path != "/" && !path.ends_with("/app") {
        candidates.push(input.clone());
    }
    candidates.dedup_by(|left, right| left.as_str() == right.as_str());
    Ok(candidates)
}

pub(crate) fn probe_status_error(status: StatusCode) -> CommandError {
    let code = if status == StatusCode::NOT_FOUND {
        ErrorCode::NotFound
    } else if status.is_client_error() && status != StatusCode::TOO_MANY_REQUESTS {
        ErrorCode::InvalidInput
    } else {
        ErrorCode::Unavailable
    };
    server_error(code, "The Stoat deployment could not be verified.")
}

async fn bounded_response_bytes(
    response: reqwest::Response,
    limit: u64,
) -> Result<Vec<u8>, CommandError> {
    if response.content_length().is_some_and(|size| size > limit) {
        return Err(server_error(
            ErrorCode::InvalidResponse,
            "The Stoat deployment returned an oversized response.",
        ));
    }
    let mut received = 0_u64;
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| {
            server_error(
                ErrorCode::InvalidResponse,
                "The Stoat deployment returned an unreadable response.",
            )
        })?;
        received = received
            .checked_add(chunk.len() as u64)
            .filter(|received| *received <= limit)
            .ok_or(server_error(
                ErrorCode::InvalidResponse,
                "The Stoat deployment returned an oversized response.",
            ))?;
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn fetch_json(
    client: &Client,
    url: Url,
    limit: u64,
) -> Result<(StatusCode, Url, Value), CommandError> {
    let response = client.get(url).send().await.map_err(|_| {
        server_error(
            ErrorCode::Unavailable,
            "The Stoat deployment could not be reached.",
        )
    })?;
    let status = response.status();
    let final_url = response.url().clone();
    let bytes = bounded_response_bytes(response, limit).await?;
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).map_err(|_| {
            server_error(
                ErrorCode::InvalidResponse,
                "The Stoat deployment returned an invalid response.",
            )
        })?
    };
    Ok((status, final_url, value))
}

async fn fetch_node_config(client: &Client, api: &Url) -> Result<(Value, Url), CommandError> {
    let (status, final_url, value) = fetch_json(client, api.clone(), MAX_DISCOVERY_BYTES).await?;
    if !status.is_success() {
        return Err(probe_status_error(status));
    }
    if value.get("ws").and_then(Value::as_str).is_none()
        || value
            .get("stoat")
            .or_else(|| value.get("revolt"))
            .and_then(Value::as_str)
            .is_none()
    {
        return Err(server_error(
            ErrorCode::InvalidResponse,
            "The deployment is not a compatible Stoat server.",
        ));
    }
    Ok((value, final_url))
}

pub(crate) fn resolve_http_reference(base: &Url, value: &str) -> Result<Url, CommandError> {
    if value.starts_with("//") {
        return Err(server_error(
            ErrorCode::InvalidResponse,
            "The deployment advertised an ambiguous service endpoint.",
        ));
    }
    let url = Url::parse(value)
        .or_else(|_| base.join(value))
        .map_err(|_| {
            server_error(
                ErrorCode::InvalidResponse,
                "The deployment advertised an invalid service endpoint.",
            )
        })?;
    if !secure_http_endpoint(&url) {
        return Err(server_error(
            ErrorCode::InvalidResponse,
            "The deployment advertised an insecure service endpoint.",
        ));
    }
    Ok(url)
}

pub(crate) fn resolve_websocket_reference(base: &Url, value: &str) -> Result<Url, CommandError> {
    if value.starts_with("//") {
        return Err(server_error(
            ErrorCode::InvalidResponse,
            "The deployment advertised an ambiguous gateway endpoint.",
        ));
    }
    let url = match Url::parse(value) {
        Ok(url) => url,
        Err(_) => {
            let mut url = base.join(value).map_err(|_| {
                server_error(
                    ErrorCode::InvalidResponse,
                    "The deployment advertised an invalid gateway endpoint.",
                )
            })?;
            let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
            url.set_scheme(scheme).map_err(|_| {
                server_error(
                    ErrorCode::InvalidResponse,
                    "The deployment advertised an invalid gateway endpoint.",
                )
            })?;
            url
        }
    };
    if !secure_websocket_endpoint(&url) {
        return Err(server_error(
            ErrorCode::InvalidResponse,
            "The deployment advertised an insecure gateway endpoint.",
        ));
    }
    Ok(url)
}

pub(crate) fn feature_service_url(
    features: &Value,
    name: &str,
    api: &Url,
) -> Result<Option<String>, CommandError> {
    let Some(feature) = features.get(name) else {
        return Ok(None);
    };
    if feature.get("enabled").and_then(Value::as_bool) == Some(false) {
        return Ok(None);
    }
    match feature.get("url").and_then(Value::as_str) {
        Some(value) => resolve_http_reference(api, value).map(|url| Some(url.to_string())),
        None if feature.get("enabled").and_then(Value::as_bool) == Some(true) => Err(server_error(
            ErrorCode::InvalidResponse,
            "The deployment enabled a service without advertising its endpoint.",
        )),
        None => Ok(None),
    }
}

pub(crate) fn well_known_api_value(base: &Url, value: &Value) -> Result<Url, CommandError> {
    let advertised = value
        .get("api")
        .and_then(Value::as_str)
        .ok_or(server_error(
            ErrorCode::InvalidResponse,
            "The deployment discovery document did not contain an API endpoint.",
        ))?;
    resolve_http_reference(base, advertised)
}

async fn well_known_api(client: &Client, input: &Url) -> Result<Option<Url>, CommandError> {
    let mut url = deployment_origin(input)?;
    url.set_path("/.well-known/stoat");
    let response = match client.get(url).send().await {
        Ok(response) => response,
        Err(_) => return Ok(None),
    };
    let status = response.status();
    if !status.is_success() {
        return Ok(None);
    }
    let final_url = response.url().clone();
    let bytes = bounded_response_bytes(response, MAX_WELL_KNOWN_BYTES).await?;
    let value = serde_json::from_slice(&bytes).map_err(|_| {
        server_error(
            ErrorCode::InvalidResponse,
            "The deployment discovery document was invalid.",
        )
    })?;
    well_known_api_value(&final_url, &value).map(Some)
}

pub(crate) fn selection_from_config(
    input: &Url,
    api: Url,
    config: &Value,
) -> Result<ServerSelection, CommandError> {
    let invalid_advertisement = || {
        server_error(
            ErrorCode::InvalidResponse,
            "The Stoat deployment advertised invalid service endpoints.",
        )
    };
    let features = config.get("features").unwrap_or(&Value::Null);
    let api_version = config
        .get("stoat")
        .or_else(|| config.get("revolt"))
        .and_then(Value::as_str)
        .filter(|version| !version.is_empty())
        .ok_or_else(invalid_advertisement)?;
    let websocket = config
        .get("ws")
        .and_then(Value::as_str)
        .filter(|endpoint| !endpoint.is_empty())
        .ok_or_else(invalid_advertisement)?;
    let official = api.scheme() == "https"
        && api.host_str() == Some("api.stoat.chat")
        && api.port_or_known_default() == Some(443);
    let origin = deployment_origin(input).map_err(|_| invalid_advertisement())?;
    let attachment_size_limit = ["new_user", "default"]
        .into_iter()
        .filter_map(|tier| {
            config
                .pointer(&format!(
                    "/limits/{tier}/file_upload_size_limits/attachments"
                ))
                .and_then(Value::as_u64)
        })
        .min()
        .or_else(|| {
            config
                .pointer("/limits/global/body_limit_size")
                .and_then(Value::as_u64)
        })
        .unwrap_or(DEFAULT_ATTACHMENT_SIZE_LIMIT);
    let selection = ServerSelection {
        deployment_url: if official {
            OFFICIAL_DEPLOYMENT.into()
        } else {
            origin.to_string().trim_end_matches('/').into()
        },
        display_name: if official {
            "Official Stoat".into()
        } else {
            origin.host_str().unwrap_or("Custom Stoat").into()
        },
        official,
        api_version: api_version.into(),
        capabilities: ServerCapabilities {
            email_verification: features
                .get("email")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            invite_only: features
                .get("invite_only")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            captcha: CaptchaCapability {
                enabled: features
                    .get("captcha")
                    .and_then(|feature| feature.get("enabled"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                site_key: features
                    .get("captcha")
                    .and_then(|feature| feature.get("key"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
            },
            attachment_size_limit,
        },
        endpoints: ServerEndpoints {
            api: api.to_string().trim_end_matches('/').into(),
            websocket: resolve_websocket_reference(&api, websocket)
                .map_err(|_| invalid_advertisement())?
                .to_string(),
            autumn: feature_service_url(features, "autumn", &api)
                .map_err(|_| invalid_advertisement())?,
            january: feature_service_url(features, "january", &api)
                .map_err(|_| invalid_advertisement())?,
            gifbox: feature_service_url(features, "gifbox", &api)
                .map_err(|_| invalid_advertisement())?
                .or_else(|| {
                    Some(if official {
                        "https://api.gifbox.me".to_owned()
                    } else {
                        format!("{}/gifbox", origin.to_string().trim_end_matches('/'))
                    })
                }),
            livekit_enabled: features
                .get("livekit")
                .and_then(|feature| feature.get("enabled"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
        },
    };
    validate_selection(&selection).map_err(|_| invalid_advertisement())?;
    Ok(selection)
}

pub async fn probe_server(deployment_url: &str) -> Result<ServerSelection, CommandError> {
    let input = normalize_input(deployment_url)?;
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(format!("Stonks/{}", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= MAX_PROBE_REDIRECTS {
                return attempt.stop();
            }
            let target = attempt.url();
            let secure_target = secure_http_endpoint(target);
            let downgrade = attempt
                .previous()
                .last()
                .is_some_and(|previous| previous.scheme() == "https" && target.scheme() != "https");
            if secure_target && !downgrade {
                attempt.follow()
            } else {
                attempt.error("the deployment redirected to an insecure endpoint")
            }
        }))
        .build()
        .map_err(|_| {
            server_error(
                ErrorCode::Unavailable,
                "The network client is temporarily unavailable.",
            )
        })?;

    let mut candidates = api_candidates(&input)?;
    if candidates
        .first()
        .is_none_or(|candidate| candidate.host_str() != Some("api.stoat.chat"))
    {
        if let Some(discovered) = well_known_api(&client, &input).await? {
            let insert_at = usize::from(candidates.first() == Some(&input));
            if !candidates.iter().any(|candidate| candidate == &discovered) {
                candidates.insert(insert_at, discovered);
            }
        }
    }
    let mut last_error = None;
    for candidate in candidates {
        match fetch_node_config(&client, &candidate).await {
            Ok((config, api)) => return selection_from_config(&input, api, &config),
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        server_error(
            ErrorCode::Unavailable,
            "No compatible Stoat API could be reached.",
        )
    }))
}
