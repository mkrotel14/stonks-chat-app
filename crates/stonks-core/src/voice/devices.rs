//! PipeWire graph discovery without opening capture or playback streams.
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiscoveryError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Input,
    Output,
    Camera,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub kind: Kind,
    pub id: String,
    pub label: String,
    pub is_default: bool,
}

pub(super) async fn graph() -> Result<Vec<Value>, DiscoveryError> {
    let output = tokio::time::timeout(
        Duration::from_secs(3),
        tokio::process::Command::new("pw-dump")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| DiscoveryError)?
    .map_err(|_| DiscoveryError)?;
    if !output.status.success() {
        return Err(DiscoveryError);
    }
    serde_json::from_slice(&output.stdout).map_err(|_| DiscoveryError)
}

pub async fn discover() -> Result<Vec<Device>, DiscoveryError> {
    Ok(parse(&graph().await?))
}

fn parse(graph: &[Value]) -> Vec<Device> {
    let default = |key: &str| {
        graph
            .iter()
            .filter(|node| node["props"]["metadata.name"] == "default")
            .flat_map(|node| node["metadata"].as_array().into_iter().flatten())
            .find(|entry| entry["key"] == key)
            .and_then(|entry| entry["value"]["name"].as_str())
    };
    let input = default("default.audio.source");
    let output = default("default.audio.sink");
    let mut devices: Vec<_> = graph
        .iter()
        .filter_map(|node| {
            if node["type"] != "PipeWire:Interface:Node" {
                return None;
            }
            let props = node.pointer("/info/props")?;
            let (kind, default) = match props["media.class"].as_str()? {
                class if class.starts_with("Audio/Source") => (Kind::Input, input),
                class if class.starts_with("Audio/Sink") => (Kind::Output, output),
                _ => return None,
            };
            let name = props["node.name"].as_str()?;
            if name.is_empty() || name.ends_with(".monitor") || props["stream.monitor"] == true {
                return None;
            }
            let label = ["node.description", "node.nick", "device.description"]
                .into_iter()
                .find_map(|key| props[key].as_str().filter(|label| !label.is_empty()))
                .unwrap_or(name);
            Some(Device {
                kind,
                id: format!("pipewire:{name}"),
                label: label.into(),
                is_default: default == Some(name),
            })
        })
        .chain(
            super::camera::devices::parse(graph)
                .into_iter()
                .map(|camera| Device {
                    kind: Kind::Camera,
                    id: camera.id,
                    label: camera.label,
                    is_default: camera.is_default,
                }),
        )
        .collect();
    devices.sort_by_cached_key(|device| {
        (
            !device.is_default,
            device.label.to_lowercase(),
            device.id.clone(),
        )
    });
    devices.dedup_by(|a, b| a.kind == b.kind && a.id == b.id);
    devices
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn discovers_routable_nodes_without_monitor_sources() {
        let node = |class, name| json!({"type": "PipeWire:Interface:Node", "info": {"props": {"media.class": class, "node.name": name}}});
        let devices = parse(&[
            json!({"props":{"metadata.name":"default"},"metadata":[{"key":"default.audio.sink","value":{"name":"speakers"}}]}),
            node("Audio/Source", "mic"),
            node("Audio/Sink", "speakers"),
            node("Audio/Source", "speakers.monitor"),
            node("Video/Source", "cam"),
        ]);
        assert_eq!(
            devices
                .iter()
                .map(|d| (d.kind, d.id.as_str(), d.is_default))
                .collect::<Vec<_>>(),
            [
                (Kind::Output, "pipewire:speakers", true),
                (Kind::Camera, "pipewire:cam", false),
                (Kind::Input, "pipewire:mic", false)
            ]
        );
    }
}
