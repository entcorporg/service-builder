#!/usr/bin/env bash
# Packs target/release/service-builder into dist/Service.Builder_<version>_amd64.AppImage, with GTK 4 bundled.
# usage: scripts/appimage.sh <tag>      (run after `cargo build --release -p service-builder`)
set -euo pipefail

tag="${1:?usage: appimage.sh <tag>}"
version="${tag#v}"
root="$(cd "$(dirname "$0")/.." && pwd)"
tools="$root/target/appimage-tools"
mkdir -p "$tools" "$root/dist"

fetch() { [ -x "$tools/$1" ] || { curl -fsSL "$2" -o "$tools/$1" && chmod +x "$tools/$1"; }; }
fetch linuxdeploy https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage
fetch linuxdeploy-plugin-gtk.sh https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gtk/master/linuxdeploy-plugin-gtk.sh

appdir="$root/target/appimage/AppDir"
rm -rf "$appdir"
data="$root/crates/sb-gtk/data"
export PATH="$tools:$PATH" DEPLOY_GTK_VERSION=4 APPIMAGE_EXTRACT_AND_RUN=1 LINUXDEPLOY_OUTPUT_VERSION="$version"
export OUTPUT="$root/dist/Service.Builder_${version}_amd64.AppImage"

"$tools/linuxdeploy" --appdir "$appdir" \
  --executable "$root/target/release/service-builder" \
  --desktop-file "$data/dev.servicebuilder.app.desktop" \
  --icon-file "$data/service-builder.png" \
  --plugin gtk --output appimage
