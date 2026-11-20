//! The gateway websocket worker. Tauri-free: hosts subscribe through the
//! [`ConnectionEvents`] sink instead of `AppHandle` event emission; the
//! Tauri shell bridges the sink onto its own events, the GPUI app onto
//! entity notifications.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::time::{self, Duration, MissedTickBehavior};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use url::Url;

use super::gateway::{gateway_event, ready_snapshot, GatewayEvent};
use super::models::{AppSnapshot, ClientUser, ConnectionStatus, ConnectionStatusReason};
use super::state::ConnectionManager;
use crate::auth::credentials::delete_token_if_matches;
use crate::http::{
    AuthenticatedContext, HttpPolicy, HttpPolicyError, RequestClass, MAX_JSON_RESPONSE_BYTES,
};
use crate::server::ServerSelection;

/// Host-side subscription to connection lifecycle and gateway traffic.
pub trait ConnectionEvents: Send + Sync {
    fn status(&self, status: ConnectionStatus);
    fn ready(&self, snapshot: AppSnapshot);
    fn gateway_event(&self, event: GatewayEvent);
}

fn connection_status(
    state: &'static str,
    reason: Option<ConnectionStatusReason>,
    attempt: u32,
) -> ConnectionStatus {
    let message = reason.map(|reason| match reason {
        ConnectionStatusReason::AuthenticationFailed => {
            "The connection could not be authenticated. Log in again."
        }
        ConnectionStatusReason::ConnectionLost => "The connection was interrupted. Retrying.",
        ConnectionStatusReason::SignedOut => "This session was signed out remotely.",
    });
    ConnectionStatus {
        state,
        reason,
        message,
        attempt,
    }
}

struct Worker {
    policy: Arc<HttpPolicy>,
    manager: Arc<ConnectionManager>,
    events: Arc<dyn ConnectionEvents>,
}

impl Worker {
    fn emit_status(
        &self,
        generation: u64,
        state: &'static str,
        reason: Option<ConnectionStatusReason>,
        attempt: u32,
    ) {
        let _ = self.manager.publish(generation, || {
            self.events
                .status(connection_status(state, reason, attempt));
            Ok(())
        });
    }

    fn emit_gateway_event(&self, generation: u64, value: Value) {
        let Some(event) = gateway_event(value) else {
            return;
        };
        let event_type = event.event_type.clone();
        let event_data = event.data.clone();
        let _ = self.manager.publish_gateway_event(
            generation,
            &event_type,
            &event_data,
            || {
                self.events.gateway_event(event);
                Ok(())
            },
        );
    }
}

async fn fetch_current_user(
    policy: &HttpPolicy,
    context: &AuthenticatedContext,
) -> Result<ClientUser, String> {
    let request = policy
        .client()
        .get(format!("{}/users/@me", context.server.endpoints.api))
        .header("x-session-token", &context.token);
    let response = policy
        .execute_authenticated(
            context,
            request,
            RequestClass::Standard,
            MAX_JSON_RESPONSE_BYTES,
        )
        .await
        .map_err(|error| match error {
            HttpPolicyError::InvalidResponse | HttpPolicyError::BodyTooLarge => {
                "The account response was invalid.".to_owned()
            }
            HttpPolicyError::Unavailable | HttpPolicyError::Timeout => {
                "Could not reach the Stoat API.".to_owned()
            }
            HttpPolicyError::Cancelled => {
                "The authenticated account request was cancelled.".to_owned()
            }
        })?;
    response.consume(
        |status, _headers, bytes| {
            if !status.is_success() {
                return Err(format!("The Stoat API returned HTTP {status}."));
            }
            let mut user: ClientUser = serde_json::from_slice(bytes)
                .map_err(|error| format!("The account response was invalid: {error}"))?;
            user.online = true;
            Ok(user)
        },
        || "The authenticated account request was cancelled.".to_owned(),
    )
}

fn loopback_url(url: &Url) -> bool {
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

fn gateway_url(server: &ServerSelection) -> Result<Url, String> {
    let api = Url::parse(&server.endpoints.api)
        .map_err(|_| "The saved API address is invalid.".to_owned())?;
    let mut url = Url::parse(&server.endpoints.websocket)
        .map_err(|_| "The server advertised an invalid WebSocket URL.".to_owned())?;
    let clean = url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none();
    let local_development = api.scheme() == "http" && loopback_url(&api);
    let valid_transport =
        url.scheme() == "wss" || (url.scheme() == "ws" && local_development && loopback_url(&url));
    if !clean || !valid_transport || (loopback_url(&url) && !loopback_url(&api)) {
        return Err("The server advertised an invalid or insecure WebSocket URL.".into());
    }
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("version", "1");
        query.append_pair("format", "json");
        for field in [
            "users",
            "servers",
            "channels",
            "members",
            "channel_unreads",
            "voice_states",
        ] {
            query.append_pair("ready", field);
        }
    }
    Ok(url)
}

async fn connection_once(
    worker: &Worker,
    context: &AuthenticatedContext,
    attempt: u32,
    generation: u64,
    outgoing: &mut mpsc::UnboundedReceiver<Value>,
) -> Result<(), String> {
    worker.emit_status(generation, "connecting", None, attempt);
    let current_user = fetch_current_user(&worker.policy, context).await?;
    let url = gateway_url(&context.server)?;
    let (socket, _) = connect_async(url.as_str())
        .await
        .map_err(|error| format!("Could not connect to the Stoat gateway: {error}"))?;
    let (mut writer, mut reader) = socket.split();
    writer
        .send(Message::Text(
            json!({ "type": "Authenticate", "token": context.token })
                .to_string()
                .into(),
        ))
        .await
        .map_err(|error| format!("Could not authenticate the gateway: {error}"))?;

    let mut ping = time::interval(Duration::from_secs(20));
    ping.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ping.tick().await;

    loop {
        tokio::select! {
            queued = outgoing.recv() => {
                let Some(value) = queued else { return Err("The gateway command queue closed.".into()); };
                writer.send(Message::Text(value.to_string().into()))
                    .await.map_err(|error| format!("Could not send a gateway command: {error}"))?;
            }
            _ = ping.tick() => {
                let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
                writer.send(Message::Text(json!({ "type": "Ping", "data": now }).to_string().into()))
                    .await.map_err(|error| format!("Gateway ping failed: {error}"))?;
            }
            incoming = reader.next() => {
                let message = incoming.ok_or("The Stoat gateway closed the connection.")?
                    .map_err(|error| format!("Gateway connection error: {error}"))?;
                let text = match message {
                    Message::Text(text) => text.to_string(),
                    Message::Binary(bytes) => String::from_utf8(bytes.to_vec()).map_err(|_| "The gateway sent non-UTF-8 data.")?,
                    Message::Close(_) => return Err("The Stoat gateway closed the connection.".into()),
                    _ => continue,
                };
                let value: Value = serde_json::from_str(&text)
                    .map_err(|error| format!("The gateway sent invalid JSON: {error}"))?;
                let event_type = value.get("type").and_then(Value::as_str).unwrap_or("Unknown");
                match event_type {
                    "Authenticated" => worker.emit_status(generation, "authenticated", None, attempt),
                    "Ready" => {
                        let snapshot = ready_snapshot(&value, current_user.clone())?;
                        let authorization = json!({ "ready": value, "currentUser": current_user });
                        let published = worker.manager.publish_ready(
                            generation,
                            authorization,
                            || {
                                worker.events.ready(snapshot);
                                Ok(())
                            },
                        )?;
                        if published {
                            worker.emit_status(generation, "ready", None, attempt);
                        }
                    }
                    "Error" => return Err(format!("Gateway authentication failed: {}", value.get("error").and_then(Value::as_str).unwrap_or("unknown error"))),
                    "Logout" => {
                        if worker.policy.logout_context(context) {
                            let _ = delete_token_if_matches(
                                &context.server,
                                &context.token,
                            );
                        }
                        worker.manager.clear_ready(generation);
                        worker.emit_status(
                            generation,
                            "loggedOut",
                            Some(ConnectionStatusReason::SignedOut),
                            attempt,
                        );
                        return Ok(());
                    }
                    "Pong" => {}
                    "Bulk" => {
                        if let Some(events) = value.get("v").and_then(Value::as_array) {
                            for event in events {
                                worker.emit_gateway_event(generation, event.clone());
                            }
                        }
                    }
                    _ => worker.emit_gateway_event(generation, value),
                }
            }
        }
    }
}

async fn connection_loop(
    worker: Worker,
    context: AuthenticatedContext,
    generation: u64,
    mut outgoing: mpsc::UnboundedReceiver<Value>,
) {
    let mut attempt = 1;
    loop {
        match connection_once(&worker, &context, attempt, generation, &mut outgoing).await {
            Ok(()) => return,
            Err(error) => {
                if error.contains("authentication failed") || error.contains("HTTP 401") {
                    worker.emit_status(
                        generation,
                        "error",
                        Some(ConnectionStatusReason::AuthenticationFailed),
                        attempt,
                    );
                    return;
                }
                worker.emit_status(
                    generation,
                    "reconnecting",
                    Some(ConnectionStatusReason::ConnectionLost),
                    attempt,
                );
                time::sleep(Duration::from_secs((attempt.min(5) * 2).into())).await;
                attempt += 1;
            }
        }
    }
}

pub fn start_connection(
    policy: Arc<HttpPolicy>,
    manager: Arc<ConnectionManager>,
    events: Arc<dyn ConnectionEvents>,
    context: AuthenticatedContext,
    owner_id: Option<String>,
) -> Result<(), String> {
    let generation = manager.begin(owner_id)?;
    let worker = Worker {
        policy,
        manager: manager.clone(),
        events,
    };
    let (outgoing, receiver) = mpsc::unbounded_channel();
    let task_manager = manager.clone();
    let task = tokio::spawn(async move {
        connection_loop(worker, context, generation, receiver).await;
        task_manager.finish(generation);
    });
    manager.install(generation, task, outgoing)
}

pub fn stop_connection(manager: &ConnectionManager, owner_id: Option<&str>) -> Result<(), String> {
    manager.stop(owner_id)
}

pub fn set_typing(
    manager: &ConnectionManager,
    channel_id: String,
    typing: bool,
) -> Result<(), String> {
    manager.send(typing_payload(channel_id, typing))
}

fn typing_payload(channel_id: String, typing: bool) -> Value {
    json!({
        "type": if typing { "BeginTyping" } else { "EndTyping" },
        "channel": channel_id,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn connection_statuses_expose_only_stable_reasons_and_safe_messages() {
        assert_eq!(
            serde_json::to_value(connection_status(
                "reconnecting",
                Some(ConnectionStatusReason::ConnectionLost),
                2,
            ))
            .unwrap(),
            json!({
                "state": "reconnecting",
                "reason": "connection_lost",
                "message": "The connection was interrupted. Retrying.",
                "attempt": 2
            })
        );
    }

    fn gateway_selection(deployment: &str, websocket: &str) -> ServerSelection {
        serde_json::from_value(json!({
            "deploymentUrl": deployment,
            "displayName": "Test",
            "official": false,
            "apiVersion": "0.16.1",
            "endpoints": {
                "api": if deployment.starts_with("http://") {
                    "http://localhost:14702/api"
                } else {
                    "https://chat.example.com/api"
                },
                "websocket": websocket,
                "autumn": null,
                "january": null,
                "gifbox": null,
                "livekitEnabled": false
            }
        }))
        .unwrap()
    }

    #[test]
    fn appends_required_gateway_query_fields() {
        let url = gateway_url(&gateway_selection(
            "https://chat.example.com",
            "wss://events.example.net/ws",
        ))
        .unwrap();
        assert!(url.as_str().contains("version=1"));
        assert!(url.as_str().contains("format=json"));
        assert!(url.as_str().contains("ready=users"));
        assert!(url.as_str().contains("ready=voice_states"));
    }

    #[test]
    fn rejects_insecure_or_ambiguous_gateway_urls_at_connection_time() {
        assert!(gateway_url(&gateway_selection(
            "http://localhost:14702",
            "ws://127.0.0.42:14702/ws",
        ))
        .is_ok());
        for websocket in [
            "ws://events.example.net/ws",
            "https://events.example.net/ws",
            "wss://person:secret@events.example.net/ws",
            "wss://events.example.net/ws?version=2",
            "wss://events.example.net/ws#fragment",
            "wss://localhost:14702/ws",
        ] {
            assert!(
                gateway_url(&gateway_selection("https://chat.example.com", websocket)).is_err()
            );
        }
        assert!(gateway_url(&gateway_selection(
            "http://localhost:14702",
            "ws://192.168.1.20:14702/ws",
        ))
        .is_err());
    }

    #[test]
    fn creates_gateway_typing_commands() {
        assert_eq!(
            typing_payload("channel".into(), true),
            json!({ "type": "BeginTyping", "channel": "channel" })
        );
        assert_eq!(
            typing_payload("channel".into(), false),
            json!({ "type": "EndTyping", "channel": "channel" })
        );
    }
}
