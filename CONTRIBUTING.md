# Contributing

Use the pinned Rust toolchain, keep lockfiles checked in, and avoid dependency upgrades unrelated to your change. Linux needs the native development libraries listed in `.github/workflows/gpui.yml` (including GStreamer 1.28); Windows uses the x64 MSVC SDK setup in `packaging/gpui/windows.ps1`.

```sh
cargo check --locked --manifest-path crates/stonks-app/Cargo.toml --all-targets
cargo test --locked --manifest-path crates/stonks-app/Cargo.toml --all-targets
cargo test --locked --manifest-path crates/stonks-core/Cargo.toml
cargo test --locked --manifest-path crates/stonks-ui/Cargo.toml
```

Run focused tests and format changed Rust. For UI changes, verify relevant themes, sizes and locales in a native window. Keep original assets and their sources reproducible: `python3 scripts/generate-project-assets.py` regenerates the original launcher art and cue WAVs. Document the source and redistribution terms of any new third-party asset before adding it.
