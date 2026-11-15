#!/usr/bin/env bash
# Construit le paquet natif d'Intact Bitstream pour la distribution courante.
# Prévu pour tourner (en root) dans un conteneur propre de la distribution
# cible : c'est ce que fait l'intégration continue GitHub.
#
# Usage : packaging/build.sh [dossier-de-sortie]      (défaut : dist/)
# Distributions : Ubuntu ≥ 24.04, Debian ≥ 13, Fedora ≥ 40, openSUSE
# Tumbleweed, Arch Linux.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT=$(mkdir -p "${1:-$ROOT/dist}" && cd "${1:-$ROOT/dist}" && pwd)
VERSION=$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)
REL=1
MIN_RUST=$(sed -n 's/^rust-version *= *"\(.*\)"/\1/p' "$ROOT/Cargo.toml")
ID=$(. /etc/os-release && echo "$ID")
VERSION_ID=$(. /etc/os-release && echo "${VERSION_ID:-}")
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
say() { printf '\n==> %s\n' "$*"; }

# Rust récent (rustup) si celui de la distribution est absent ou trop ancien.
ensure_rust() {
  export PATH="$HOME/.cargo/bin:$PATH"
  local v
  v=$(rustc --version 2>/dev/null | awk '{print $2}') || true
  if [ -n "$v" ] && [ "$(printf '%s\n%s\n' "$MIN_RUST" "$v" | sort -V | head -1)" = "$MIN_RUST" ]; then
    return
  fi
  say "Installation de Rust (rustup)"
  curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable >/dev/null
}

compile() {
  say "Compilation d'Intact Bitstream $VERSION"
  cd "$ROOT"
  cargo build --release --locked
  cargo test --release --locked
}

build_deb() {
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -q
  apt-get install -yq --no-install-recommends build-essential pkg-config libgtk-4-dev libadwaita-1-dev \
    curl ca-certificates dpkg-dev
  ensure_rust
  compile
  local tag="${ID}${VERSION_ID}" ver arch stage
  ver="$VERSION-$REL~$tag"
  arch=$(dpkg --print-architecture)
  stage="$WORK/pkg"
  "$ROOT/packaging/stage.sh" "$stage"

  # Dépendances calculées depuis les bibliothèques réellement liées.
  mkdir -p "$WORK/shlib/debian"
  printf 'Source: intact-bitstream\n\nPackage: intact-bitstream\nArchitecture: any\n' > "$WORK/shlib/debian/control"
  local shlibs
  shlibs=$(cd "$WORK/shlib" && dpkg-shlibdeps -O "$stage/usr/bin/intact-bitstream" 2>/dev/null | sed -n 's/^shlibs:Depends=//p')

  mkdir -p "$stage/DEBIAN"
  cat > "$stage/DEBIAN/control" <<CTRL
Package: intact-bitstream
Version: $ver
Architecture: $arch
Maintainer: Intact Bitstream <https://github.com/loganxwolverine/intact-bitstream>
Installed-Size: $(du -sk "$stage/usr" | cut -f1)
Depends: $shlibs, flatpak, mkvtoolnix, hicolor-icon-theme
Recommends: eject
Section: video
Priority: optional
Homepage: https://github.com/loganxwolverine/intact-bitstream
Description: Vos DVD et Blu-ray en MKV, sans perte
 Interface GTK4/libadwaita qui pilote MakeMKV : reconnaissance du film avec
 son affiche, choix automatique des pistes, nommage complet du fichier et
 éjection en fin de copie. MakeMKV est installé automatiquement depuis Flathub.
CTRL
  cat > "$stage/DEBIAN/postinst" <<'POST'
#!/bin/sh
set -e
if [ "$1" = configure ]; then
  modprobe sg >/dev/null 2>&1 || true
  udevadm control --reload-rules >/dev/null 2>&1 || true
  udevadm trigger --subsystem-match=scsi_generic >/dev/null 2>&1 || true
fi
POST
  chmod 755 "$stage/DEBIAN/postinst"
  local file="$OUT/intact-bitstream_${ver}_${arch}.deb"
  dpkg-deb --root-owner-group --build "$stage" "$file"
  say "Paquet : $file"
}

build_rpm() {
  local dist
  case "$ID" in
    fedora)
      dnf install -y gcc pkgconf-pkg-config gtk4-devel libadwaita-devel rpm-build curl
      dist=".fc${VERSION_ID}" ;;
    opensuse*)
      zypper --non-interactive install gcc pkgconf-pkg-config gtk4-devel libadwaita-devel rpm-build curl tar gzip
      dist=".suse.tw" ;;
    *) echo "Distribution RPM non prise en charge : $ID" >&2; exit 1 ;;
  esac
  ensure_rust
  compile
  local stage="$WORK/pkg"
  "$ROOT/packaging/stage.sh" "$stage"
  rpmbuild -bb "$ROOT/packaging/rpm/intact-bitstream.spec" \
    --define "_topdir $WORK/rpmbuild" \
    --define "ver $VERSION" --define "rel $REL$dist" --define "stage $stage"
  find "$WORK/rpmbuild/RPMS" -name '*.rpm' -exec cp -v {} "$OUT/" \;
}

build_arch() {
  pacman -Syu --needed --noconfirm base-devel rust gtk4 libadwaita flatpak mkvtoolnix-cli hicolor-icon-theme
  # makepkg refuse de tourner en root : utilisateur dédié.
  id builder >/dev/null 2>&1 || useradd -m builder
  local dir="$WORK/arch"
  mkdir -p "$dir"
  cp "$ROOT/packaging/arch/PKGBUILD" "$dir/"
  sed -i "s/^pkgver=.*/pkgver=$VERSION/" "$dir/PKGBUILD"
  # Sources locales à la place de l'archive GitHub (même nom = pas de téléchargement).
  tar --exclude=./target --exclude=./dist --exclude=./.git \
      --transform "s,^\.,intact-bitstream-$VERSION," -czf "$dir/intact-bitstream-$VERSION.tar.gz" -C "$ROOT" .
  chown -R builder "$dir"
  chmod 755 "$WORK"
  su builder -c "cd '$dir' && PKGDEST='$dir/out' makepkg --noconfirm"
  cp -v "$dir"/out/*.pkg.tar.zst "$OUT/"
  cp "$dir/PKGBUILD" "$OUT/PKGBUILD"
}

case "$ID" in
  ubuntu|debian) build_deb ;;
  fedora|opensuse*) build_rpm ;;
  arch) build_arch ;;
  *) echo "Distribution non prise en charge : $ID" >&2; exit 1 ;;
esac
ls -l "$OUT"
