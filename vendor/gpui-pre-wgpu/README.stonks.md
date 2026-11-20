# Stonks GPUI renderer patch

Source: crates.io `gpui-pre-wgpu` 0.3.12 (Zed `d2a4fb6` snapshot), used by `gpui-kit` 0.9.1 with `wgpu` 30.0.1. The Apache license is retained. The local policy lives in `src/native_adapter_policy.rs` and is applied in `src/wgpu_context.rs`; the app's manifest patches this crate so the Linux platform and startup caller share one policy.

Linux startup should call `gpui_wgpu::set_native_adapter_policy(policy)` once before opening any GPUI windows: `NativeAdapterPolicy::Automatic` for acceleration enabled, `NativeAdapterPolicy::SoftwareOnly` for disabled. Handle `Err(active_policy)` as a startup ordering/configuration error. Changes require an app restart; the renderer defaults to Automatic if no policy was supplied.

The pinned upstream renderer enumerates Vulkan and GL adapters and validates them against the actual window surface. `ZED_DEVICE_ID` only changes ranking, and a backend name does not distinguish hardware from software. The patch preserves the upstream ranking and surface probe, but software-only mode restricts both startup and device-loss recovery to adapters wgpu classifies as `Cpu`. It does not install a software driver or silently fall back to hardware: if no CPU adapter can configure the surface, window creation fails. Startup integration should report this error and offer a way to reset the preference before opening a window again. This policy does not affect headless renderers, other operating systems, WebViews, or video decoding.

Focused tests: `cargo test --locked --manifest-path crates/stonks-app/Cargo.toml -p gpui-pre-wgpu --test native_adapter_policy`.
