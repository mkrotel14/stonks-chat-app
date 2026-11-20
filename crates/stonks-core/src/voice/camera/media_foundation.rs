#[cfg(windows)]
use super::{CameraDevice, CameraError};

struct Device {
    id: String,
    label: String,
}

fn gst_unquote(raw: &str) -> String {
    let value = raw.trim();
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        let mut unescaped = String::with_capacity(value.len() - 2);
        let mut chars = value[1..value.len() - 1].chars();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                match chars.next() {
                    Some(escaped) => unescaped.push(escaped),
                    None => unescaped.push('\\'),
                }
            } else {
                unescaped.push(ch);
            }
        }
        unescaped
    } else {
        value.to_owned()
    }
}

#[derive(Default)]
struct GstMonitorBlock {
    name: Option<String>,
    is_video_source: bool,
    is_media_foundation: bool,
    device_path: Option<String>,
}

impl GstMonitorBlock {
    fn take(&mut self, devices: &mut Vec<Device>) {
        let block = std::mem::take(self);
        if block.is_video_source && block.is_media_foundation {
            if let (Some(label), Some(id)) = (block.name, block.device_path) {
                if id.is_empty() || devices.iter().any(|device| device.id == id) {
                    return;
                }
                devices.push(Device { id, label });
            }
        }
    }
}

/// Parses `gst-device-monitor-1.0 Video/Source` output. Media Foundation
/// devices are the ones `mfvideosrc` can open; their `device.path` property
/// is the ID the element's `device-path` property consumes.
fn parse_gst_media_foundation_cameras(output: &str) -> Vec<Device> {
    let mut devices = Vec::new();
    let mut block = GstMonitorBlock::default();
    for line in output.lines() {
        let line = line.trim();
        if line == "Device found:" {
            block.take(&mut devices);
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            match key.trim() {
                "name" => block.name = Some(value.trim().to_owned()),
                "class" => block.is_video_source = value.trim() == "Video/Source",
                _ => {}
            }
        }
        if let Some((key, value)) = line.split_once('=') {
            match key.trim() {
                "device.api" => block.is_media_foundation = value.trim() == "mediafoundation",
                "device.path" | "device-path" => block.device_path = Some(gst_unquote(value)),
                _ => {}
            }
        }
    }
    block.take(&mut devices);
    devices
}

#[cfg(windows)]
pub(super) async fn devices() -> Result<Vec<CameraDevice>, CameraError> {
    let mut command = tokio::process::Command::new("gst-device-monitor-1.0");
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(6),
        command.arg("Video/Source").kill_on_drop(true).output(),
    )
    .await
    .map_err(|_| CameraError::Discovery)?
    .map_err(|_| CameraError::Discovery)?;
    if !output.status.success() {
        return Err(CameraError::Discovery);
    }
    Ok(
        parse_gst_media_foundation_cameras(&String::from_utf8_lossy(&output.stdout))
            .into_iter()
            .enumerate()
            .map(|(index, device)| CameraDevice {
                id: device.id,
                label: device.label,
                is_default: index == 0,
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_media_foundation_video_paths_are_selectable() {
        let devices = parse_gst_media_foundation_cameras(
            r#"
Device found:
  name: Legacy Camera
  class: Video/Source
  device.api = directshow
  device.path = legacy
Device found:
  name: USB Camera
  class: Video/Source
  device.api = mediafoundation
  device.path = "\\\\?\\usb"
Device found:
  name: Missing Path
  class: Video/Source
  device.api = mediafoundation
"#,
        );
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id, r"\\?\usb");
        assert_eq!(devices[0].label, "USB Camera");
    }
}
