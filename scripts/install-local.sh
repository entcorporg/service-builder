#!/usr/bin/env bash
# Builds Service Builder here and replaces the AppImage in ~/Applications with it: the app is built on this machine,
# not by CI.
#
#   scripts/install-local.sh            build, pack, install into ~/Applications/Service.Builder.AppImage
#   SB_APPLICATIONS=/some/dir scripts/install-local.sh    install somewhere else
#
# The running app keeps working while it is replaced (the new file takes the name, the old one lives on until the
# window closes): restart it to get the new version. Needs the Rust toolchain, curl, and the tools of
# scripts/appimage.sh (it downloads linuxdeploy once, into target/appimage-tools).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

version="$(sed -n '/^\[workspace.package\]/,/^\[/{s/^version *= *"\(.*\)"/\1/p}' Cargo.toml | head -n1)"
[ -n "$version" ] || { echo "no version in Cargo.toml" >&2; exit 1; }
apps="${SB_APPLICATIONS:-$HOME/Applications}"
target="$apps/Service.Builder.AppImage"

echo "▸ Compilation de Service Builder $version…"
cargo build --release -p service-builder

echo "▸ AppImage…"
scripts/appimage.sh "v$version"
image="$root/dist/Service.Builder_${version}_amd64.AppImage"
[ -f "$image" ] || { echo "AppImage not produced: $image" >&2; exit 1; }

echo "▸ Installation dans $target…"
mkdir -p "$apps"
# Written next to the target, then renamed: never a half-written AppImage under the real name.
cp "$image" "$target.new"
chmod +x "$target.new"
mv -f "$target.new" "$target"
echo "✓ $target ($version, $(du -h "$target" | cut -f1)). Relancez l'application pour l'utiliser."
