#!/usr/bin/env bash
# Build Linux release artifacts: a portable tarball and, when dpkg-deb is
# available, a Debian package.
#
# Usage: package.sh <binary> <version> <output-dir> [arch]
#   arch: x86_64 (default) or aarch64
set -euo pipefail

binary=${1:?binary path required}
version=${2:?version required}
out_dir=${3:?output directory required}
arch=${4:-x86_64}

case "$arch" in
  x86_64) deb_arch=amd64 ;;
  aarch64) deb_arch=arm64 ;;
  *) echo "unsupported arch: $arch" >&2; exit 1 ;;
esac

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
pkg_name="tabby-rust-$version-linux-$arch"
stage="$out_dir/deb-stage-$arch"
rm -rf "$stage"
mkdir -p "$stage/usr/bin" \
         "$stage/usr/share/applications" \
         "$stage/usr/share/icons/hicolor/scalable/apps" \
         "$stage/usr/share/doc/tabby-rust"

install -m 0755 "$binary" "$stage/usr/bin/tabby-rust"
install -m 0644 "$script_dir/tabby-rust.desktop" "$stage/usr/share/applications/tabby-rust.desktop"
install -m 0644 "$script_dir/tabby-rust.svg" \
  "$stage/usr/share/icons/hicolor/scalable/apps/tabby-rust.svg"
install -m 0644 "$root/README.md" "$root/LICENSE" "$stage/usr/share/doc/tabby-rust/"

# Portable tarball keeps the same payload as the deb but under one top-level
# directory, so extracting it never scatters files into the current folder.
portable="$out_dir/$pkg_name"
rm -rf "$portable"
mkdir -p "$portable"
install -m 0755 "$binary" "$portable/tabby-rust"
install -m 0644 "$root/README.md" "$root/LICENSE" "$portable/"
install -m 0644 "$script_dir/tabby-rust.svg" "$portable/tabby-rust.svg"
tarball="$out_dir/$pkg_name.tar.gz"
tar -C "$out_dir" -czf "$tarball" "$pkg_name"
echo "wrote $tarball"

deb_root=""
if command -v dpkg-deb >/dev/null 2>&1; then
  deb_root="$out_dir/deb-$arch"
  rm -rf "$deb_root"
  cp -a "$stage/." "$deb_root/"
  mkdir -p "$deb_root/DEBIAN"
  cat > "$deb_root/DEBIAN/control" <<EOF
Package: tabby-rust
Version: $version
Section: utils
Priority: optional
Architecture: $deb_arch
Maintainer: Tabby Rust contributors
Depends: libc6, libfontconfig1, libfreetype6, libxkbcommon0, libxkbcommon-x11-0, libxcb1, libwayland-client0, libvulkan1
Recommends: openssh-client
Description: Tabby-compatible terminal emulator and SSH client
 A Rust reimplementation of Tabby built on GPUI. Provides tabs and splits,
 local shells, SSH and telnet sessions, Tabby-compatible profiles and color
 schemes.
EOF
  deb="$out_dir/tabby-rust-$version-linux-$arch.deb"
  dpkg-deb --root-owner-group --build "$deb_root" "$deb" >/dev/null
  echo "wrote $deb"
else
  echo "dpkg-deb not found; skipping Debian package" >&2
fi

# Leave only the deliverables in the output directory.
rm -rf "$stage" "$portable"
if [[ -n "$deb_root" ]]; then
  rm -rf "$deb_root"
fi
