# Native GPUI CI artifacts

`.github/workflows/gpui.yml` runs for GPUI-related pull requests and main branch pushes, or by manual dispatch. It tests the checked-out revision and uploads run-scoped artifacts only. It does not publish releases or updater assets. Versions come from `crates/stonks-app/Cargo.toml`.

## Linux x86_64

CI compiles and tests with Cargo `--locked` inside a digest-pinned Arch container with packages from a dated Arch Linux Archive snapshot. The runtime floor includes GStreamer 1.28 and the snapshot's glibc ABI. On a compatible Arch host with the CI dependencies installed:

```sh
cargo test --locked --manifest-path crates/stonks-app/Cargo.toml --all-targets
cargo build --locked --release --manifest-path crates/stonks-app/Cargo.toml --bin stonks-app
VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json \
  bash packaging/gpui/linux.sh crates/stonks-app/target/release/stonks-app /tmp/gpui-artifacts
```

Both packages carry the same release binary, desktop entry, icon and notices. The script validates ELF architecture and linkage, checks pacman metadata, extracts both packages and compares payloads, and requires a visible window from each packaged launcher under Xvfb. The AppImage runs in extract-and-run mode for CI; FUSE mounting is not tested. The archived AppImage runtime is fetched by asset ID and checked by SHA-256, so if upstream removes the asset the build fails closed.

- `stonks-gpui-<version>-1-x86_64.pkg.tar.zst`: install with `pacman -U`; runtime packages are declared in the recipe.
- `Stonks.GPUI_<version>_x86_64.AppImage`: a thin AppImage for hosts compatible with that same Arch baseline. It uses system libraries, Vulkan and GStreamer plugins; it is **not** a standalone cross-distribution bundle. Use the Arch recipe as the host dependency list. Use `--appimage-extract-and-run` when FUSE is unavailable.
- `SHA256SUMS`: checksums of both artifacts.

## Windows x86_64 (unverified until native CI passes)

The Windows runner installs the hash-verified MSVC GStreamer 1.28 SDK and pkgconf, then checks and links the actual `stonks-app.exe`. Only then does it run the Inno Setup recipe (ISCC comes from the `windows-2025` runner image; the script logs its version). The smoke gate silently installs into a temporary directory, compares the installed app, notices and four media executables against the build inputs, runs each installed tool by absolute path with its version flag, requires a visible `Stonks Chat` window from the installed app, and checks that uninstall removes the app and tools. Any failure blocks upload. The gate does not test actual playback, YouTube resolution, authenticated use or voice, and it runs on an SDK-equipped runner rather than a clean Windows machine.

The installer is unsigned and per-user, with a separate GPUI install directory and Start Menu entry. It does not replace Tauri's protocol registration or install GStreamer. Users need the x64 MSVC GStreamer 1.28 runtime and its plugins installed separately and its `bin` directory on `PATH`; the smoke test supplies that same prerequisite. The Windows installer carries `ffmpeg.exe`, `ffprobe.exe`, `yt-dlp.exe` and `deno.exe` in `resources/media` beside the app (no global `PATH` changes for those tools). Provisioning uses `scripts/media-checksums.ps1`: yt-dlp `2026.08.19` and Deno `v2.9.4` are version-pinned, while both FFmpeg programs come from one SHA-256-verified BtbN `ffmpeg-master-latest-win64-lgpl.zip` archive. The FFmpeg URL is rolling, so builds are not reproducible from those URLs alone; upstream asset or checksum changes can block packaging. The checksums come from the same upstream release endpoints as the downloads. The Linux recipes do not bundle these tools.
