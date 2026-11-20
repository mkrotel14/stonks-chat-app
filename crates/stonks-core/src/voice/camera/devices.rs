use super::CameraError;
#[cfg(target_os = "linux")]
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraDevice {
    pub id: String,
    pub label: String,
    pub is_default: bool,
    #[cfg(target_os = "linux")]
    source: Source,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(target_os = "linux")]
enum Source {
    V4l2(String),
    PipeWire(String),
}

#[cfg(target_os = "linux")]
impl CameraDevice {
    pub(super) fn source_args(&self) -> [&str; 2] {
        match &self.source {
            Source::V4l2(arg) => ["v4l2src", arg],
            Source::PipeWire(arg) => ["pipewiresrc", arg],
        }
    }
}

/// Reads graph metadata only; never opens a camera stream.
#[cfg(target_os = "linux")]
pub async fn devices() -> Result<Vec<CameraDevice>, CameraError> {
    Ok(parse(
        &super::super::devices::graph()
            .await
            .map_err(|_| CameraError::Discovery)?,
    ))
}

#[cfg(target_os = "linux")]
pub(in crate::voice) fn parse(graph: &[Value]) -> Vec<CameraDevice> {
    let default = graph
        .iter()
        .filter(|node| node["props"]["metadata.name"] == "default")
        .flat_map(|node| node["metadata"].as_array().into_iter().flatten())
        .find(|entry| entry["key"] == "default.video.source")
        .and_then(|entry| entry["value"]["name"].as_str());
    let mut devices: Vec<_> = graph
        .iter()
        .filter_map(|node| {
            if node["type"] != "PipeWire:Interface:Node" {
                return None;
            }
            let props = node.pointer("/info/props")?;
            let name = props["node.name"]
                .as_str()
                .filter(|name| !name.is_empty())?;
            if props["media.class"] != "Video/Source"
                || name.ends_with(".monitor")
                || props["stream.monitor"] == true
            {
                return None;
            }
            let label = ["node.description", "node.nick", "device.description"]
                .into_iter()
                .find_map(|key| props[key].as_str().filter(|s| !s.is_empty()))
                .unwrap_or(name);
            // Direct V4L2 preserves hardware MJPEG modes hidden by some PipeWire nodes.
            let path = props["api.v4l2.path"].as_str().filter(|path| {
                props["device.api"] == "v4l2"
                    && path.strip_prefix("/dev/video").is_some_and(|suffix| {
                        !suffix.is_empty() && suffix.bytes().all(|c| c.is_ascii_digit())
                    })
            });
            Some(CameraDevice {
                id: format!("pipewire:{name}"),
                label: label.into(),
                is_default: default == Some(name),
                source: path.map_or_else(
                    || Source::PipeWire(format!("target-object={name}")),
                    |path| Source::V4l2(format!("device={path}")),
                ),
            })
        })
        .collect();
    devices.sort_by_cached_key(|device| {
        (
            !device.is_default,
            device.label.to_lowercase(),
            device.id.clone(),
        )
    });
    devices
}

#[cfg(target_os = "windows")]
pub async fn devices() -> Result<Vec<CameraDevice>, CameraError> {
    super::media_foundation::devices().await
}

pub fn resolve_device<'a>(
    devices: &'a [CameraDevice],
    id: &str,
) -> Result<&'a CameraDevice, CameraError> {
    let device = if id == "default" {
        devices.first()
    } else {
        devices.iter().find(|device| device.id == id)
    };
    device.ok_or(CameraError::DeviceMissing)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn discovers_video_only_and_resolves_default_without_substituting_explicit_selection() {
        let node = |props: Value| json!({"type":"PipeWire:Interface:Node", "info":{"props":props}});
        let devices = parse(&[
            json!({"props":{"metadata.name":"default"},"metadata":[{"key":"default.video.source","value":{"name":"usb"}}]}),
            node(
                json!({"media.class":"Video/Source","node.name":"virtual","node.nick":"Alpha","device.api":"v4l2","api.v4l2.path":"/dev/video../bad"}),
            ),
            node(
                json!({"media.class":"Video/Source","node.name":"usb","device.api":"v4l2","api.v4l2.path":"/dev/video12"}),
            ),
            node(json!({"media.class":"Video/Source","node.name":"monitor","stream.monitor":true})),
            node(json!({"media.class":"Audio/Source","node.name":"mic"})),
        ]);
        assert_eq!(devices.len(), 2);
        assert_eq!(
            resolve_device(&devices, "default").unwrap().source_args(),
            ["v4l2src", "device=/dev/video12"]
        );
        assert_eq!(
            resolve_device(&devices, "pipewire:virtual")
                .unwrap()
                .source_args(),
            ["pipewiresrc", "target-object=virtual"]
        );
        assert_eq!(
            resolve_device(&devices, "pipewire:missing"),
            Err(CameraError::DeviceMissing)
        );
        assert_eq!(
            resolve_device(&[], "default"),
            Err(CameraError::DeviceMissing)
        );
    }
}
