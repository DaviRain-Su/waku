#!/usr/bin/env bash

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

target_dir="${CARGO_TARGET_DIR:-target}"
version="$(cargo metadata --no-deps --format-version 1 | sed -n 's/.*"name":"proofship","version":"\([^"]*\)".*/\1/p')"
target_triple="$(rustc -vV | sed -n 's/^host: //p')"
package="proofship-${version}-${target_triple}"
archive="$target_dir/release/$package.tar.gz"
staging="$(mktemp -d)"
trap 'rm -rf -- "$staging"' EXIT

cargo build --locked --release --package proofship --bin proofship --package proofship-daemon --bin proofship-daemon --package proofship-pf-mcp --bin proofship-pf-mcp

package_dir="$staging/$package"
install -Dm755 "$target_dir/release/proofship" "$package_dir/bin/proofship"
install -Dm755 "$target_dir/release/proofship-daemon" "$package_dir/bin/proofship-daemon"
install -Dm755 "$target_dir/release/proofship-pf-mcp" "$package_dir/bin/proofship-pf-mcp"
install -Dm644 resources/linux/sh.proofship.desktop \
  "$package_dir/share/applications/sh.proofship.desktop"
install -Dm644 website/public/app-icon.png \
  "$package_dir/share/icons/hicolor/256x256/apps/sh.proofship.png"
install -Dm644 LICENSE "$package_dir/share/licenses/proofship/LICENSE"

mkdir -p "$(dirname "$archive")"
tar -C "$staging" -czf "$archive" "$package"
printf 'Created %s\n' "$archive"
