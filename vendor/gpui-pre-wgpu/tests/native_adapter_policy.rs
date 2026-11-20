#![cfg(target_os = "linux")]

// Test the production module without compiling GPUI's unrelated library tests.
#[path = "../src/native_adapter_policy.rs"]
mod native_adapter_policy;

#[test]
fn startup_policy_cannot_be_changed_after_selection() {
    use native_adapter_policy::{
        NativeAdapterPolicy::*, native_adapter_policy, set_native_adapter_policy,
    };

    assert_eq!(set_native_adapter_policy(SoftwareOnly), Ok(()));
    assert_eq!(native_adapter_policy(), SoftwareOnly);
    assert_eq!(set_native_adapter_policy(Automatic), Err(SoftwareOnly));
}
