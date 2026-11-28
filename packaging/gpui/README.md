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

All three packages carry the same release binary, desktop entry, icon and notices. The script validates ELF architecture and linkage, checks pacman metadata, compares extracted payloads, and requires a visible window from each launcher under Xvfb. CI runs the `arch` and `appimage` modes separately, uploading `gpui-arch-x86_64` before the AppImage step starts, then `gpui-appimage-x86_64` if it passes. The AppImage runs in extract-and-run mode for CI; FUSE mounting is not tested. The runtime download tries the upstream release URL first and the pinned GitHub asset ID second. Both paths require the same pinned SHA-256; a changed upstream runtime cannot silently enter the build.

- `stonks-gpui-<version>-1-x86_64.pkg.tar.zst`: install with `pacman -U`; runtime packages are declared in the recipe.
- `Stonks.GPUI_<version>_x86_64.AppImage`: a thin AppImage for hosts compatible with that same Arch baseline. It uses system libraries, Vulkan and GStreamer plugins; it is **not** a standalone cross-distribution bundle. Use the Arch recipe as the host dependency list. Use `--appimage-extract-and-run` when FUSE is unavailable.
- `stonks-<version>-1-x86_64.pkg.tar.zst`: continuation of the GPUI bridge package. Preserves the `stonks` package identity and `/usr/bin/stonks` launcher. Do not install it alongside `stonks-gpui`.
- `SHA256SUMS`: checksums of the packages built by the selected mode (`all` by default).

### In-app updates

On startup, managed Fedora and Windows installations automatically start the update flow when the release check finds a compatible newer package. Fedora still prompts through Polkit; Windows still opens the per-user Setup UI. Failure offers Retry or Continue. Arch keeps the in-app confirmation, and AppImage remains manual.

Desktop Settings → Updates checks the latest stable release in `mkrotel14/stonks-chat-app`. For updates to reach both package identities, attach both unmodified Arch artifacts to that GitHub release, with a tag of `v<version>` (or `<version>`) matching the packaged version. CI artifacts alone are not an update channel. The updater requires the matching asset's GitHub-provided SHA-256 digest and byte size; it does not use `SHA256SUMS` or bypass local pacman signature policy. This trusts GitHub and the pinned repository's release publishers, not an independent package signature.

Only x86-64 Arch installs whose running binary is `/usr/bin/stonks-app` and is owned by one of the supported packages can install in-app. The user must confirm the exact package/version before downloading and then authorize `pkexec` via a desktop polkit agent. Pacman updates the same package identity with no forced overwrite. The app requests a restart after confirming the replacement package version at the installed executable path. AppImage and unmanaged builds remain discovery-only.

## Fedora 44 x86_64 native RPM

`gpui-fedora-44-x86_64` contains `stonks-gpui-<version>-1.fc44.x86_64.rpm` and `SHA256SUMS`. On Fedora 44, install with `sudo dnf install ./stonks-gpui-<version>-1.fc44.x86_64.rpm`. Fedora 44's signed release repository has GStreamer 1.28.1; Fedora 43's updates are still on 1.26 and do not meet the build floor. No third-party repositories are needed for the RPM's hard dependencies.

CI uses a digest-pinned Fedora 44 base and enables only the signed `fedora` release repository for build and runtime validation. It uses the pinned Rust toolchain, Cargo `--locked`, and checks the three GStreamer SDK modules. The RPM automatically declares linked ELF libraries and explicitly requires runtime plugins and tools that linking cannot detect. It uses Fedora's available plugins; this does not imply coverage of all codecs. External music/extraction tools are optional and not bundled. The pins are not a guarantee of byte-for-byte reproducibility.

On a managed Fedora 44 x86-64 install (RPM owns `/usr/bin/stonks-app`), the updater selects only the matching `stonks-gpui-<version>-<revision>.fc44.x86_64.rpm` release asset. It verifies the digest, RPM identity and executable before running `pkexec /usr/bin/dnf install -y <verified-package>`. It restarts only after RPM reports the exact selected package version. A polkit authentication agent is required; the updater does not change DNF signature policy.

A fresh container installs through DNF with weak dependencies disabled, verifies RPM metadata/payload and ELF linkage, checks required GStreamer elements and executable tools, then uses D-Bus, Xvfb and Mesa software Vulkan to require a window from `/usr/bin/stonks-app` that stays visible across multiple checks. The smoke environment works around a Fedora 44 Lavapipe shader-compiler crash; the RPM does not set these driver overrides. It also checks that uninstall removes the binary. This is an X11 startup smoke test, not coverage of Wayland, hardware GPUs, actual media playback, voice, screen sharing, or authenticated use. It does not validate the Arch-built AppImage on Fedora or imply support for other distributions.

## Windows x86_64 (unverified until native CI passes)

The Windows runner installs the hash-verified MSVC GStreamer 1.28 SDK and pkgconf, then checks and links the actual `stonks-app.exe`. Only then does it run the Inno Setup recipe (ISCC comes from the `windows-2025` runner image; the script logs its version). The smoke gate silently installs into a temporary directory, compares the installed app, notices and four media executables against the build inputs, runs each installed tool by absolute path with its version flag, requires a visible `Stonks Chat` window from the installed app, and checks that uninstall removes the app and tools. Any failure blocks upload. The gate does not test actual playback, YouTube resolution, authenticated use or voice, and it runs on an SDK-equipped runner rather than a clean Windows machine.

The installer is unsigned and per-user, with a separate GPUI install directory and Start Menu entry. It does not replace Tauri's protocol registration or install GStreamer. Users need the x64 MSVC GStreamer 1.28 runtime and its plugins installed separately and its `bin` directory on `PATH`; the smoke test supplies that same prerequisite. The Windows installer carries `ffmpeg.exe`, `ffprobe.exe`, `yt-dlp.exe` and `deno.exe` in `resources/media` beside the app (no global `PATH` changes for those tools). Provisioning uses `scripts/media-checksums.ps1`: yt-dlp `2026.08.19` and Deno `v2.9.4` are version-pinned, while both FFmpeg programs come from one SHA-256-verified BtbN `ffmpeg-master-latest-win64-lgpl.zip` archive. The FFmpeg URL is rolling, so builds are not reproducible from those URLs alone; upstream asset or checksum changes can block packaging. The checksums come from the same upstream release endpoints as the downloads. The Linux recipes do not bundle these tools.

For Windows in-app updates, attach the unmodified `Stonks.GPUI_<version>_x64-setup.exe` built with this recipe to the matching stable release. Only the matching per-user GPUI Inno registration and running version qualify. GPUI verifies the download and launches Setup with `/UPDATE`, the current process ID and registered version, then quits normally. Setup waits for GPUI to exit, checks that the registered version and install directory have not changed, and relaunches GPUI after a successful install. Cancellation or failure does not relaunch the app; reopen it from Start. A `stonks-update-*` staging directory is retained so Inno can reopen the verified Setup executable after handoff; it can be removed once Setup has exited.
