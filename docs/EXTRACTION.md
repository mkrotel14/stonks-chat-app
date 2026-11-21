# Standalone extraction

Source snapshot: `stonks-tauri` tracked files at `2ddd0da2ae92b6fd67f202873de23d22a262e9b4`. The copy was made from Git blobs, not from the working tree. It does not inherit Git metadata.

Allowlist: `.cargo/`, `.gitattributes`, `rust-toolchain.toml`, `crates/{stonks-app,stonks-core,stonks-ui}/`, `vendor/{gpui-pre-wgpu,livekit,livekit-protocol,webrtc-sys}/`, `packaging/gpui/`, `.github/workflows/gpui.yml`, `scripts/{media-checksums.ps1,generate-native-inline-font.py}`, `docs/{GPUI_COMPACT_NAVIGATION,GPUI_PERFORMANCE}.md`, and `src/locales/{en,pt-BR}.json` (relocated to `crates/stonks-ui/assets/locales/`). The original launcher PNG was excluded and replaced. No legacy sounds or brand assets were copied. Historical comparisons in the retained notes are not build dependencies.

The local launcher and tray images restore the original Stonks artwork, identified by the project owner as AI-generated. The cue generator does not generate or overwrite that artwork.
