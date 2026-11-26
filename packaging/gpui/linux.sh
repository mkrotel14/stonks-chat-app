#!/usr/bin/env bash
set -euo pipefail

repo=$(realpath "$(dirname "$0")/../..")
binary=$(realpath "${1:?Pass the native release binary}")
output=$(realpath -m "${2:?Pass the artifact directory}")
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

arch=$work/arch
appdir=$work/Stonks.AppDir
mkdir -p "$arch" "$appdir" "$output"
install -m755 "$binary" "$arch/stonks-app"
cp "$repo/crates/stonks-app/launcher/"{"$id.desktop","$id.png"} "$arch/"
cp "$repo/"{LICENSE,THIRD_PARTY_NOTICES.md} "$arch/"
desktop-file-validate "$arch/$id.desktop"
sed "s/^pkgver=0$/pkgver=$version/" "$repo/packaging/gpui/PKGBUILD" > "$arch/PKGBUILD"

# makepkg refuses to run as root; the hosted container starts as root.
builder=()
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
bridge=$work/bridge
cp -a "$arch" "$bridge"
sed -i 's/^pkgname=stonks-gpui$/pkgname=stonks/' "$bridge/PKGBUILD"
(
  cd "$bridge"
  "${builder[@]}" makepkg --noconfirm --clean
)
bridge_package=$bridge/stonks-$version-1-x86_64.pkg.tar.zst
test "$(pacman -Qp "$bridge_package")" = "stonks $version-1"
mkdir "$work/bridge-tree"
bsdtar -xf "$bridge_package" -C "$work/bridge-tree"
cmp "$binary" "$work/bridge-tree/usr/bin/stonks-app"
test "$(readlink "$work/bridge-tree/usr/bin/stonks")" = stonks-app

package=$arch/stonks-gpui-$version-1-x86_64.pkg.tar.zst
test "$(pacman -Qp "$package")" = "stonks-gpui $version-1"
mkdir "$work/arch-tree"
bsdtar -xf "$package" -C "$work/arch-tree"
arch_tree=$work/arch-tree
cmp "$binary" "$arch_tree/usr/bin/stonks-app"
cmp "$arch/$id.desktop" "$arch_tree/usr/share/applications/$id.desktop"
cmp "$arch/$id.png" "$arch_tree/usr/share/icons/hicolor/128x128/apps/$id.png"
cmp "$arch/THIRD_PARTY_NOTICES.md" "$arch_tree/usr/share/licenses/stonks-gpui/THIRD_PARTY_NOTICES.md"

install -Dm755 "$arch/stonks-app" "$appdir/usr/bin/stonks-app"
install -Dm644 "$arch/LICENSE" "$appdir/usr/share/doc/stonks-gpui/LICENSE"
install -Dm644 "$arch/THIRD_PARTY_NOTICES.md" "$appdir/usr/share/doc/stonks-gpui/THIRD_PARTY_NOTICES.md"
cp "$arch/"{"$id.desktop","$id.png"} "$appdir/"
ln -s "$id.png" "$appdir/.DirIcon"
cat > "$appdir/AppRun" <<'EOF'
#!/bin/sh
exec "$(dirname "$0")/usr/bin/stonks-app" "$@"
EOF
chmod +x "$appdir/AppRun"

fetch() {
  curl --fail --silent --show-error --location --retry 3 "$1" -o "$2" "${@:4}"
  printf '%s  %s\n' "$3" "$2" | sha256sum --check --strict
}
fetch https://github.com/AppImage/appimagetool/releases/download/1.9.0/appimagetool-x86_64.AppImage \
  "$work/appimagetool" 46fdd785094c7f6e545b61afcfb0f3d98d8eab243f644b4b17698c01d06083d1
fetch https://api.github.com/repos/AppImage/type2-runtime/releases/assets/657374641 \
  "$work/runtime" 617aa77314d57c8b8e4fae9c3f0b35af7b30d5529cb69e98f64f92510ed98a46 \
  -H 'Accept: application/octet-stream'
chmod +x "$work/appimagetool"
image=$work/Stonks.GPUI_${version}_x86_64.AppImage
ARCH=x86_64 "$work/appimagetool" --appimage-extract-and-run --runtime-file "$work/runtime" "$appdir" "$image"
(
  cd "$work"
  "$image" --appimage-extract >/dev/null
)
cmp "$binary" "$work/squashfs-root/usr/bin/stonks-app"
cmp "$arch/$id.desktop" "$work/squashfs-root/$id.desktop"
cmp "$arch/$id.png" "$work/squashfs-root/$id.png"
cmp "$arch/THIRD_PARTY_NOTICES.md" "$work/squashfs-root/usr/share/doc/stonks-gpui/THIRD_PARTY_NOTICES.md"

# A visible window, not merely a surviving process. Use host Mesa software
# Vulkan and isolated desktop state; no account or secrets are required.
for launcher in "$arch_tree/usr/bin/stonks-app" "$work/bridge-tree/usr/bin/stonks" "$image"; do
  dbus-run-session -- xvfb-run -a -s '-screen 0 1280x800x24' \
    bash "$repo/packaging/gpui/smoke-linux.sh" "$launcher"
done
install -m755 "$image" "$output/"
install -m644 "$package" "$bridge_package" "$output/"
(cd "$output"; sha256sum "$(basename "$image")" "$(basename "$package")" "$(basename "$bridge_package")" > SHA256SUMS)
