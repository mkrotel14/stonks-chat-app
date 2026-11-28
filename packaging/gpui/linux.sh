#!/usr/bin/env bash
set -euo pipefail

repo=$(realpath "$(dirname "$0")/../..")
binary=$(realpath "${1:?Pass the native release binary}")
output=$(realpath -m "${2:?Pass the artifact directory}")
mode=${3:-all}
case "$mode" in
  all|arch|appimage) ;;
  *) echo 'Expected all, arch or appimage' >&2; exit 2 ;;
esac
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
id=com.rdxlabs.stonks.gpui
version=$(python -c 'import sys,tomllib; print(tomllib.load(open(sys.argv[1], "rb"))["package"]["version"])' "$repo/crates/stonks-app/Cargo.toml")
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]
test -x "$binary"
readelf -h "$binary" | grep -q 'Advanced Micro Devices X86-64'
check_ldd() {
  local linkage
  linkage=$(ldd "$1")
  if grep -q 'not found' <<< "$linkage"; then
    printf '%s\n' "$linkage" >&2
    return 1
  fi
}
check_ldd "$binary"

launcher_assets=$repo/crates/stonks-app/launcher
desktop-file-validate "$launcher_assets/$id.desktop"
mkdir -p "$output"

# A visible window, not merely a surviving process. Use host Mesa software
# Vulkan and isolated desktop state; no account or secrets are required.
smoke() {
  dbus-run-session -- xvfb-run -a -s '-screen 0 1280x800x24' \
    bash "$repo/packaging/gpui/smoke-linux.sh" "$1" --require-stable-window
}

build_arch() {
  local arch=$work/arch bridge=$work/bridge arch_tree=$work/arch-tree
  local package=stonks-gpui-$version-1-x86_64.pkg.tar.zst
  local bridge_package=stonks-$version-1-x86_64.pkg.tar.zst
  mkdir "$arch"
  install -m755 "$binary" "$arch/stonks-app"
  cp "$launcher_assets/"{"$id.desktop","$id.png"} "$arch/"
  cp "$repo/"{LICENSE,THIRD_PARTY_NOTICES.md} "$arch/"
  sed "s/^pkgver=0$/pkgver=$version/" "$repo/packaging/gpui/PKGBUILD" > "$arch/PKGBUILD"

  # makepkg refuses to run as root; the hosted container starts as root.
  local -a builder=()
  if (( EUID == 0 )); then
    chmod 755 "$work"
    chown -R nobody "$arch"
    builder=(runuser -u nobody --)
  fi
  (
    cd "$arch"
    "${builder[@]}" makepkg -g >> checksums
    cat checksums >> PKGBUILD
    "${builder[@]}" makepkg --noconfirm --clean
  )
  # Build a same-identity update for existing bridge installs, not a replacement.
  cp -a "$arch" "$bridge"
  sed -i 's/^pkgname=stonks-gpui$/pkgname=stonks/' "$bridge/PKGBUILD"
  (
    cd "$bridge"
    "${builder[@]}" makepkg --noconfirm --clean
  )
  test "$(pacman -Qp "$bridge/$bridge_package")" = "stonks $version-1"
  mkdir "$work/bridge-tree"
  bsdtar -xf "$bridge/$bridge_package" -C "$work/bridge-tree"
  cmp "$binary" "$work/bridge-tree/usr/bin/stonks-app"
  test "$(readlink "$work/bridge-tree/usr/bin/stonks")" = stonks-app

  test "$(pacman -Qp "$arch/$package")" = "stonks-gpui $version-1"
  mkdir "$arch_tree"
  bsdtar -xf "$arch/$package" -C "$arch_tree"
  cmp "$binary" "$arch_tree/usr/bin/stonks-app"
  cmp "$arch/$id.desktop" "$arch_tree/usr/share/applications/$id.desktop"
  cmp "$arch/$id.png" "$arch_tree/usr/share/icons/hicolor/128x128/apps/$id.png"
  cmp "$arch/THIRD_PARTY_NOTICES.md" "$arch_tree/usr/share/licenses/stonks-gpui/THIRD_PARTY_NOTICES.md"

  smoke "$arch_tree/usr/bin/stonks-app"
  smoke "$work/bridge-tree/usr/bin/stonks"
  install -m644 "$arch/$package" "$bridge/$bridge_package" "$output/"
  artifacts+=("$package" "$bridge_package")
}

fetch() {
  rm -f -- "$2"
  curl --fail --silent --show-error --location --proto '=https' --retry 3 \
    --connect-timeout 15 --max-time 120 "$1" -o "$2" "${@:4}" &&
    printf '%s  %s\n' "$3" "$2" | sha256sum --check --strict
}

build_appimage() {
  local appdir=$work/Stonks.AppDir image=Stonks.GPUI_${version}_x86_64.AppImage
  local runtime_sha=617aa77314d57c8b8e4fae9c3f0b35af7b30d5529cb69e98f64f92510ed98a46
  mkdir "$appdir"
  install -Dm755 "$binary" "$appdir/usr/bin/stonks-app"
  install -Dm644 "$repo/LICENSE" "$appdir/usr/share/doc/stonks-gpui/LICENSE"
  install -Dm644 "$repo/THIRD_PARTY_NOTICES.md" "$appdir/usr/share/doc/stonks-gpui/THIRD_PARTY_NOTICES.md"
  cp "$launcher_assets/"{"$id.desktop","$id.png"} "$appdir/"
  ln -s "$id.png" "$appdir/.DirIcon"
  cat > "$appdir/AppRun" <<'EOF'
#!/bin/sh
exec "$(dirname "$0")/usr/bin/stonks-app" "$@"
EOF
  chmod +x "$appdir/AppRun"

  fetch https://github.com/AppImage/appimagetool/releases/download/1.9.0/appimagetool-x86_64.AppImage \
    "$work/appimagetool" 46fdd785094c7f6e545b61afcfb0f3d98d8eab243f644b4b17698c01d06083d1
  # The browser URL avoids the API rate limit but can change when upstream
  # rolls its continuous release. Only accept the pinned bytes from either URL.
  if ! fetch https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-x86_64 \
    "$work/runtime" "$runtime_sha"; then
    echo 'Direct runtime download failed verification; trying the pinned GitHub asset ID' >&2
    fetch https://api.github.com/repos/AppImage/type2-runtime/releases/assets/657374641 \
      "$work/runtime" "$runtime_sha" -H 'Accept: application/octet-stream'
  fi
  chmod +x "$work/appimagetool"
  ARCH=x86_64 "$work/appimagetool" --appimage-extract-and-run --runtime-file "$work/runtime" "$appdir" "$work/$image"
  (
    cd "$work"
    "./$image" --appimage-extract >/dev/null
  )
  cmp "$binary" "$work/squashfs-root/usr/bin/stonks-app"
  cmp "$appdir/$id.desktop" "$work/squashfs-root/$id.desktop"
  cmp "$appdir/$id.png" "$work/squashfs-root/$id.png"
  cmp "$repo/THIRD_PARTY_NOTICES.md" "$work/squashfs-root/usr/share/doc/stonks-gpui/THIRD_PARTY_NOTICES.md"
  smoke "$work/$image"
  install -m755 "$work/$image" "$output/"
  artifacts+=("$image")
}

artifacts=()
if [[ $mode != appimage ]]; then build_arch; fi
if [[ $mode != arch ]]; then build_appimage; fi
(cd "$output"; sha256sum -- "${artifacts[@]}" > SHA256SUMS)
