#!/usr/bin/env bash
set -euo pipefail

repo=$(realpath "$(dirname "$0")/../..")
id=com.rdxlabs.stonks.gpui
output=$(realpath "${2:?Pass the artifact directory}")
# Use only the signed Fedora 44 release compose, not the rolling updates repo.
dnf=(dnf -y --repo=fedora --setopt=install_weak_deps=False)
. /etc/os-release
[[ $ID == fedora && $VERSION_ID == 44 && $(uname -m) == x86_64 ]]

case ${1:?Pass build or smoke} in
  build)
    "${dnf[@]}" install rustup gcc gcc-c++ clang cmake ninja-build make binutils \
      git curl unzip python3 rpm-build redhat-rpm-config desktop-file-utils pkgconf-pkg-config \
      alsa-lib-devel dbus-devel wayland-devel libxkbcommon-x11-devel \
      libxcb-devel fontconfig-devel glib2-devel pipewire-devel \
      gstreamer1-devel gstreamer1-plugins-base-devel
    cd "$repo"
    toolchain=$(python3 -c 'import tomllib; print(tomllib.load(open("rust-toolchain.toml", "rb"))["toolchain"]["channel"])')
    # Fedora's RPM ships rustup-init; bootstrap the proxies for this non-login shell.
    /usr/bin/rustup-init -y --no-modify-path --default-toolchain none
    export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
    rustup toolchain install "$toolchain" --profile minimal --component clippy --component rustfmt
    clang++ --version
    for module in gstreamer-1.0 gstreamer-app-1.0 gstreamer-video-1.0; do
      pkg-config --modversion "$module"
      pkg-config --atleast-version=1.28 "$module"
    done
    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT
    export CARGO_TARGET_DIR=$work/target CARGO_BUILD_JOBS=2
    export CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0
    manifest=crates/stonks-app/Cargo.toml
    rustup run "$toolchain" cargo test --locked --manifest-path "$manifest" --all-targets
    rustup run "$toolchain" cargo build --locked --release --manifest-path "$manifest" --bin stonks-app
    version=$(python3 -c 'import sys,tomllib; print(tomllib.load(open(sys.argv[1], "rb"))["package"]["version"])' "$manifest")
    [[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]
    readelf -h "$CARGO_TARGET_DIR/release/stonks-app" | grep 'Advanced Micro Devices X86-64'
    mkdir "$work/SOURCES"
    cp "$CARGO_TARGET_DIR/release/stonks-app" "$repo/crates/stonks-app/launcher/"{"$id.desktop","$id.png"} \
      "$repo/"{LICENSE,THIRD_PARTY_NOTICES.md} "$work/SOURCES/"
    desktop-file-validate "$work/SOURCES/$id.desktop"
    rpmbuild -bb --define "_topdir $work" --define "stonks_version $version" "$repo/packaging/gpui/stonks-gpui.spec"
    package=$work/RPMS/x86_64/stonks-gpui-$version-1.fc44.x86_64.rpm
    test "$(rpm -qp --qf '%{NAME} %{VERSION} %{RELEASE} %{ARCH}' "$package")" = "stonks-gpui $version 1.fc44 x86_64"
    rpm -qpR "$package"
    sha256sum "$CARGO_TARGET_DIR/release/stonks-app" | cut -d ' ' -f 1 > "$output/binary.sha256"
    install -m644 "$package" "$output/"
    ;;
  smoke)
    packages=("$output"/*.rpm)
    [[ ${#packages[@]} == 1 && -f ${packages[0]} ]]
    package=${packages[0]}
    # Resolve package dependencies before installing the test harness.
    "${dnf[@]}" install "$package"
    "${dnf[@]}" install diffutils
    rpm -V stonks-gpui
    printf '%s  /usr/bin/stonks-app\n' "$(cat "$output/binary.sha256")" | sha256sum --check --strict
    cmp "$repo/crates/stonks-app/launcher/$id.desktop" "/usr/share/applications/$id.desktop"
    cmp "$repo/crates/stonks-app/launcher/$id.png" "/usr/share/icons/hicolor/128x128/apps/$id.png"
    for file in LICENSE THIRD_PARTY_NOTICES.md; do
      cmp "$repo/$file" "/usr/share/licenses/stonks-gpui/$file"
    done
    linkage=$(ldd /usr/bin/stonks-app)
    printf '%s\n' "$linkage"
    if grep -q 'not found' <<< "$linkage"; then exit 1; fi
    for tool in gst-launch-1.0 pw-cat pw-dump pw-link; do command -v "$tool"; done
    gst-inspect-1.0 --version
    for element in playbin3 appsink videoconvert autoaudiosink pipewiresrc v4l2src; do
      gst-inspect-1.0 "$element" >/dev/null
    done
    "${dnf[@]}" install dbus-daemon xorg-x11-server-Xvfb xorg-x11-xauth xdotool mesa-vulkan-drivers
    shopt -s nullglob
    drivers=(/usr/share/vulkan/icd.d/lvp_icd*.json)
    [[ ${#drivers[@]} == 1 ]]
    # A 128-bit vector width alone still allows AVX2 gathers in Lavapipe's LLVM JIT.
    # Cap the JIT at SSE2 and retain nopt; the optimized path can crash even with the cap.
    VK_DRIVER_FILES=${drivers[0]} GALLIUM_OVERRIDE_CPU_CAPS=sse2 \
      LP_NATIVE_VECTOR_WIDTH=128 GALLIVM_PERF=nopt \
      dbus-run-session -- xvfb-run -a -s '-screen 0 1280x800x24' \
        bash "$repo/packaging/gpui/smoke-linux.sh" /usr/bin/stonks-app --require-stable-window
    dnf -y remove stonks-gpui
    test ! -e /usr/bin/stonks-app
    ;;
  *) echo 'Expected build or smoke' >&2; exit 2 ;;
esac
