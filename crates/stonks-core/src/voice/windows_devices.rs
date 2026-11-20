//! WASAPI endpoint discovery and selection use the same CPAL IDs.
use rodio::cpal::{
    self,
    traits::{DeviceTrait, HostTrait},
};

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

pub async fn discover() -> Result<Vec<Device>, DiscoveryError> {
    tokio::task::spawn_blocking(|| {
        let host = cpal::default_host();
        let mut devices = Vec::new();
        for (kind, default, endpoints) in [
            (
                Kind::Input,
                host.default_input_device(),
                host.input_devices()
                    .map_err(|_| DiscoveryError)?
                    .collect::<Vec<_>>(),
            ),
            (
                Kind::Output,
                host.default_output_device(),
                host.output_devices().map_err(|_| DiscoveryError)?.collect(),
            ),
        ] {
            let default = default.and_then(|d| d.id().ok());
            for endpoint in endpoints {
                if let (Ok(id), Ok(description)) = (endpoint.id(), endpoint.description()) {
                    devices.push(Device {
                        kind,
                        is_default: default.as_ref() == Some(&id),
                        id: id.to_string(),
                        label: description.name().to_owned(),
                    });
                }
            }
        }
        Ok(devices)
    })
    .await
    .map_err(|_| DiscoveryError)?
}

pub(super) fn select(id: &str, input: bool) -> Option<cpal::Device> {
    let host = cpal::default_host();
    if id.is_empty() || id == "default" {
        return if input {
            host.default_input_device()
        } else {
            host.default_output_device()
        };
    }
    // Don't silently record a different microphone if a stored endpoint vanished.
    if input {
        host.input_devices()
            .ok()?
            .find(|device| matches_id(device, id))
    } else {
        host.output_devices()
            .ok()?
            .find(|device| matches_id(device, id))
    }
}

fn matches_id(device: &cpal::Device, id: &str) -> bool {
    device
        .id()
        .is_ok_and(|candidate| candidate.to_string() == id)
}
