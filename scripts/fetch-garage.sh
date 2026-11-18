#!/usr/bin/env bash
# Puts the Garage binary (the store of the projects, packed in the AppImage) at target/garage-bin/garage, and prints its
# path. It is the static binary of the official release: taken from the official image when Docker is there, else downloaded.
set -euo pipefail
version="v2.0.0"
root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/target/garage-bin/garage"
if [ ! -x "$out" ]; then
  mkdir -p "$(dirname "$out")"
  tmp="$out.part"
  if command -v docker >/dev/null 2>&1 && cid="$(docker create "dxflrs/garage:$version" 2>/dev/null)"; then
    docker cp "$cid:/garage" "$tmp" >/dev/null
    docker rm "$cid" >/dev/null
  else
    curl -fsSL "https://garagehq.deu.re/_releases/$version/x86_64-unknown-linux-musl/garage" -o "$tmp"
  fi
  chmod +x "$tmp"
  "$tmp" --version >/dev/null || { rm -f "$tmp"; echo "garage does not run" >&2; exit 1; }
  mv "$tmp" "$out"
fi
echo "$out"
