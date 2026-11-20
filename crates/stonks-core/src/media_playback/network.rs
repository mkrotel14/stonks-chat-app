use super::*;
use std::time::Duration;
use url::Url;
pub fn validate_url(value: &str) -> Result<Url, PlaybackError> {
    if value.len() > 8192 || value.chars().any(char::is_control) {
        return Err(PlaybackError::InvalidUrl);
    }
    let mut url = Url::parse(value).map_err(|_| PlaybackError::InvalidUrl)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(PlaybackError::InvalidUrl);
    }
    url.set_fragment(None);
    Ok(url)
}
pub(super) fn validate_headers(headers: &[(String, String)]) -> Result<(), PlaybackError> {
    if headers.len() > 2 {
        return Err(PlaybackError::InvalidHeaders);
    }
    let mut seen = std::collections::HashSet::new();
    for (name, value) in headers {
        let name = name.to_ascii_lowercase();
        if !matches!(name.as_str(), "user-agent" | "referer")
            || !seen.insert(name.clone())
            || value.len() > 8192
            || value.chars().any(char::is_control)
        {
            return Err(PlaybackError::InvalidHeaders);
        }
        if name == "referer" {
            validate_url(value).map_err(|_| PlaybackError::InvalidHeaders)?;
        }
    }
    Ok(())
}
/// Resolve for an explicit player load or source audio metadata preload. Redirects are validated
/// one hop at a time; GStreamer subsequently receives only the resolved URI.
pub async fn resolve_source(mut source: Source) -> Result<Source, PlaybackError> {
    validate_headers(&source.headers)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| PlaybackError::Unavailable)?;
    let mut url = validate_url(&source.url)?;
    for _ in 0..=3 {
        let send = |head: bool| {
            let mut request = if head {
                client.head(url.clone())
            } else {
                client
                    .get(url.clone())
                    .header(reqwest::header::RANGE, "bytes=0-0")
            };
            for (name, value) in &source.headers {
                request = request.header(name, value);
            }
            request
        };
        let mut response = send(true)
            .send()
            .await
            .map_err(|_| PlaybackError::Unavailable)?;
        // Signed GET URLs and small self-hosted file handlers can reject HEAD
        // while still allowing an ordinary media range request.
        if matches!(response.status().as_u16(), 403 | 404 | 405 | 501) {
            response = send(false)
                .send()
                .await
                .map_err(|_| PlaybackError::Unavailable)?;
        }
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or(PlaybackError::Unavailable)?;
            url = validate_url(
                url.join(location)
                    .map_err(|_| PlaybackError::InvalidUrl)?
                    .as_str(),
            )?;
            continue;
        }
        if !response.status().is_success() {
            return Err(PlaybackError::Unavailable);
        }
        source.url = url.to_string();
        return Ok(source);
    }
    Err(PlaybackError::Unavailable)
}
