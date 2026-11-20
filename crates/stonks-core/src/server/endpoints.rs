//! Endpoint rules: HTTPS/WSS except for loopback development, no credentials,
//! query or fragment in endpoints, and full selection validation.

use url::Url;

use crate::error::{CommandError, ErrorCode};

use super::models::ServerSelection;

const fn server_error(code: ErrorCode, message: &'static str) -> CommandError {
    CommandError::new(code, message)
}

pub(crate) fn loopback_host(url: &Url) -> bool {
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

fn clean_endpoint(url: &Url) -> bool {
    url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
}

pub(crate) fn secure_http_endpoint(url: &Url) -> bool {
    clean_endpoint(url)
        && (url.scheme() == "https" || (url.scheme() == "http" && loopback_host(url)))
}

pub(crate) fn secure_websocket_endpoint(url: &Url) -> bool {
    clean_endpoint(url) && (url.scheme() == "wss" || (url.scheme() == "ws" && loopback_host(url)))
}

fn insecure_http_endpoint(url: &Url) -> bool {
    url.scheme() == "http"
}

fn insecure_websocket_endpoint(url: &Url) -> bool {
    url.scheme() == "ws"
}

pub fn normalize_input(value: &str) -> Result<Url, CommandError> {
    let url = Url::parse(value.trim()).map_err(|_| {
        server_error(
            ErrorCode::InvalidInput,
            "Enter a complete HTTP or HTTPS deployment URL.",
        )
    })?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(server_error(
            ErrorCode::InvalidInput,
            "Enter a complete HTTP or HTTPS deployment URL.",
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(server_error(
            ErrorCode::InvalidInput,
            "Do not include credentials in the deployment address.",
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(server_error(
            ErrorCode::InvalidInput,
            "Do not include a query or fragment in the deployment address.",
        ));
    }
    if !secure_http_endpoint(&url) {
        return Err(server_error(
            ErrorCode::InvalidInput,
            "Use HTTPS for remote deployments. Plain HTTP is allowed only for loopback development.",
        ));
    }
    Ok(url)
}

pub(crate) fn deployment_origin(url: &Url) -> Result<Url, CommandError> {
    let origin = url.origin().ascii_serialization();
    Url::parse(&origin).map_err(|_| {
        server_error(
            ErrorCode::InvalidInput,
            "The deployment address has no usable origin.",
        )
    })
}

pub fn validate_selection(selection: &ServerSelection) -> Result<(), CommandError> {
    let deployment = Url::parse(&selection.deployment_url).map_err(|_| {
        server_error(
            ErrorCode::InvalidInput,
            "The server selection contains an invalid endpoint.",
        )
    })?;
    let api = Url::parse(&selection.endpoints.api).map_err(|_| {
        server_error(
            ErrorCode::InvalidInput,
            "The server selection contains an invalid endpoint.",
        )
    })?;
    let websocket = Url::parse(&selection.endpoints.websocket).map_err(|_| {
        server_error(
            ErrorCode::InvalidInput,
            "The server selection contains an invalid endpoint.",
        )
    })?;
    let loopback_deployment = loopback_host(&deployment);
    let insecure_loopback_deployment = insecure_http_endpoint(&deployment) && loopback_deployment;
    let invalid_http = |value: &str| match Url::parse(value) {
        Ok(url) => {
            !secure_http_endpoint(&url)
                || (insecure_http_endpoint(&url) && !insecure_loopback_deployment)
                || (loopback_host(&url) && !loopback_deployment)
        }
        Err(_) => true,
    };
    let invalid_websocket = !secure_websocket_endpoint(&websocket)
        || (insecure_websocket_endpoint(&websocket) && !insecure_loopback_deployment)
        || (loopback_host(&websocket) && !loopback_deployment);

    if !secure_http_endpoint(&deployment)
        || invalid_http(api.as_str())
        || invalid_websocket
        || selection
            .endpoints
            .autumn
            .as_deref()
            .is_some_and(invalid_http)
        || selection
            .endpoints
            .january
            .as_deref()
            .is_some_and(invalid_http)
        || selection
            .endpoints
            .gifbox
            .as_deref()
            .is_some_and(invalid_http)
    {
        return Err(server_error(
            ErrorCode::InvalidInput,
            "The server selection contains an invalid or insecure endpoint.",
        ));
    }
    Ok(())
}
