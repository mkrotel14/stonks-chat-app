# Stonks Chat

Stonks Chat is a native [GPUI](https://www.gpui.rs/) desktop client for Stoat. It uses Rust for the interface and application logic, and LiveKit/WebRTC for calls and media. There is no bundled webview or JavaScript frontend. External video previews open in your browser.

## Download

See [GitHub Releases](https://github.com/mkrotel14/stonks-chat-app/releases) for downloads. This repository's CI builds an Arch Linux x86_64 package and a thin AppImage. The AppImage needs compatible host libraries, Vulkan, and GStreamer 1.28; it isn't a universal Linux bundle. See the [packaging guide](packaging/gpui/README.md) for dependencies. Windows is under validation; a failed build does not produce a release installer.

## What you can do

- Browse servers, DMs, groups, and Saved Notes. Search, sort, or pin conversations and manage friend requests.
- Read and write messages with replies, reactions, mentions, search, pins, and attachments from the picker, clipboard, or drag and drop.
- Join voice calls, adjust another participant's microphone volume locally, play soundboard and join clips, and use the shared `@Music` queue.
- Share a camera or screen and open remote video in a separate resizable viewer. Linux screen capture uses the Wayland portal and PipeWire; available capture and audio options depend on the platform and installed runtime dependencies.
- Use the wide desktop layout or the compact layout with Messages, Servers, and You tabs. Configure profile, privacy, voice devices, appearance, language, keybindings, and desktop preferences.

## Architecture

| Location | Owns |
| --- | --- |
| `crates/stonks-app` | GPUI windows, application and session wiring, desktop integration, and user-facing features. |
| `crates/stonks-core` | Stoat HTTP and gateway clients, models, preferences, attachment transport, and native voice and media. |
| `crates/stonks-ui` | Shared components, theme tokens, typography, icons, locales, and markdown rendering. |
| `vendor` | Patched upstream dependencies and their separate notices and licenses. |
| `packaging/gpui` | Native package recipes and launch smoke tests. |

Features own their screens and workflows. For example, `features/conversation/` owns the timeline, composer, and conversation media; `features/voice/` owns the call dock, soundboard, Music, and capture setup. Keep feature-specific views, state, actions, and tests together. Put code in a shared module only when multiple features use it. See [AGENTS.md](AGENTS.md) for the repository rules and the [compact navigation notes](docs/GPUI_COMPACT_NAVIGATION.md) for the Android-inspired route model.

### Design system

Start with `crates/stonks-ui/src/theme.rs`, `components/`, `icons.rs`, and `i18n.rs`. Reuse semantic colors, spacing, typography, dialog and menu controls, and localized labels rather than building one-off controls in feature code. Check dark and light themes, compact and wide layouts, keyboard focus, and reduced motion when changing UI. Use [the performance notes](docs/GPUI_PERFORMANCE.md) when working on rendering and media.

## Build from source

Install the platform dependencies in the [packaging guide](packaging/gpui/README.md) and use the pinned `rust-toolchain.toml`.

```sh
cargo run --locked --manifest-path crates/stonks-app/Cargo.toml --bin stonks-app
cargo test --locked --manifest-path crates/stonks-app/Cargo.toml --all-targets
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for focused checks and contribution guidance.

## Releases and migration

CI produces run-scoped artifacts. Public downloads are published separately to this repository's Releases page. GPUI checks stable release metadata here; it does not download or install updates yet.

Legacy Tauri clients check a different feed and do not automatically discover this repository. To test migration, keep the legacy install and back up its app data, quit it, install the GPUI package separately, and verify session restore, preferences, and everyday workflows before removing the old app. A tested in-place installer upgrade is a separate release gate. Do not treat a matching executable name or version as proof that an upgrade is compatible.

## License

Project code is [Apache-2.0](LICENSE). Vendored code, fonts, emoji, and media tools retain their own terms; see [third-party notices](THIRD_PARTY_NOTICES.md).
