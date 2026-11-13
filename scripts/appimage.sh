#!/usr/bin/env bash
# Packs target/release/service-builder into dist/Service.Builder_<version>_amd64.AppImage.
# The window is egui over wgpu (Vulkan, Metal, GL): the graphics libraries (libvulkan, libGL, libxkbcommon, Wayland, X11)
# are opened at run time from the system, like every desktop has them; nothing else needs bundling.
# usage: scripts/appimage.sh <tag>      (run after `cargo build --release -p service-builder`)
set -euo pipefail

tag="${1:?usage: appimage.sh <tag>}"
version="${tag#v}"
root="$(cd "$(dirname "$0")/.." && pwd)"
tools="$root/target/appimage-tools"
mkdir -p "$tools" "$root/dist"

fetch() { [ -x "$tools/$1" ] || { curl -fsSL "$2" -o "$tools/$1" && chmod +x "$tools/$1"; }; }
fetch linuxdeploy https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage

appdir="$root/target/appimage/AppDir"
rm -rf "$appdir"
data="$root/crates/sb-egui/data"
# linuxdeploy names the icon after its file, which must match Icon= of the desktop file.
icon="$root/target/appimage/icon"
mkdir -p "$icon" && cp "$data/service-builder-512.png" "$icon/service-builder.png"
export PATH="$tools:$PATH" APPIMAGE_EXTRACT_AND_RUN=1 LINUXDEPLOY_OUTPUT_VERSION="$version"
export OUTPUT="$root/dist/Service.Builder_${version}_amd64.AppImage"

"$tools/linuxdeploy" --appdir "$appdir" \
  --executable "$root/target/release/service-builder" \
  --desktop-file "$data/dev.servicebuilder.app.desktop" \
  --icon-file "$icon/service-builder.png" \
  --output appimage
