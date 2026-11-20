/// Startup policy for native GPUI windows using the wgpu renderer.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NativeAdapterPolicy {
    /// Preserve upstream adapter ranking and device-loss recovery behavior.
    #[default]
    Automatic,
    /// Only use adapters that wgpu identifies as CPU devices.
    SoftwareOnly,
}

#[cfg(target_os = "linux")]
static NATIVE_ADAPTER_POLICY: std::sync::OnceLock<NativeAdapterPolicy> = std::sync::OnceLock::new();

/// Set once before opening the first GPUI window. Returns the existing policy
/// if it has already been set or the renderer has initialized it to Automatic.
#[cfg(target_os = "linux")]
pub fn set_native_adapter_policy(policy: NativeAdapterPolicy) -> Result<(), NativeAdapterPolicy> {
    let mut installed = false;
    let active = NATIVE_ADAPTER_POLICY.get_or_init(|| {
        installed = true;
        policy
    });
    if installed { Ok(()) } else { Err(*active) }
}

#[cfg(target_os = "linux")]
pub(crate) fn adapter_allowed(
    policy: NativeAdapterPolicy,
    device: wgpu::DeviceType,
    recovering: bool,
) -> bool {
    match policy {
        NativeAdapterPolicy::Automatic => !recovering || device != wgpu::DeviceType::Cpu,
        NativeAdapterPolicy::SoftwareOnly => device == wgpu::DeviceType::Cpu,
    }
}

pub(crate) fn native_adapter_policy() -> NativeAdapterPolicy {
    *NATIVE_ADAPTER_POLICY.get_or_init(NativeAdapterPolicy::default)
}

#[cfg(all(test, target_os = "linux"))]
mod native_policy_tests {
    use super::{NativeAdapterPolicy, adapter_allowed};
    use wgpu::DeviceType;

    #[test]
    fn software_only_never_uses_hardware_even_during_recovery() {
        for recovering in [false, true] {
            for device in [
                DeviceType::DiscreteGpu,
                DeviceType::IntegratedGpu,
                DeviceType::VirtualGpu,
                DeviceType::Other,
            ] {
                assert!(!adapter_allowed(
                    NativeAdapterPolicy::SoftwareOnly,
                    device,
                    recovering
                ));
            }
            assert!(adapter_allowed(
                NativeAdapterPolicy::SoftwareOnly,
                DeviceType::Cpu,
                recovering
            ));
        }
    }

    #[test]
    fn automatic_keeps_upstream_fallback_and_recovery_rules() {
        assert!(adapter_allowed(
            NativeAdapterPolicy::Automatic,
            DeviceType::Cpu,
            false
        ));
        assert!(!adapter_allowed(
            NativeAdapterPolicy::Automatic,
            DeviceType::Cpu,
            true
        ));
        assert!(adapter_allowed(
            NativeAdapterPolicy::Automatic,
            DeviceType::DiscreteGpu,
            true
        ));
    }
}
