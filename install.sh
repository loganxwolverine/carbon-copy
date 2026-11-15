#!/usr/bin/env bash
# Intact Bitstream — installation automatique.
#
#   curl -fsSL https://raw.githubusercontent.com/loganxwolverine/intact-bitstream/main/install.sh | bash
#
# Le script installe TOUT ce qui est nécessaire, avec une seule intervention :
# votre mot de passe administrateur (sudo), demandé une fois au début.
#
#   1. paquets système : bibliothèques GTK 4 / libadwaita, compilateur,
#      Flatpak, MKVToolNix, eject ;
#   2. Rust (via rustup, dans votre dossier personnel) si la version de la
#      distribution est trop ancienne ;
#   3. compilation d'Intact Bitstream et installation dans ~/.local (menu + icône) ;
#   4. MakeMKV depuis Flathub (en mode utilisateur) ;
#   5. accès au lecteur : module « sg » (Blu-ray) et groupe du lecteur ;
#   6. lancement de l'application.
#
# Distributions prises en charge : Fedora 40+, Ubuntu 24.04+ (Mint 22,
# Pop!_OS 24.04, elementary OS 8), Debian 13+, Arch et dérivées,
# openSUSE Tumbleweed / Leap 16.
#
# Options :
#   --uninstall      retire l'application (garde vos réglages)
#   --purge          avec --uninstall : supprime aussi réglages et sources
#   --no-launch      ne lance pas l'application à la fin
#   --skip-system    n'installe aucun paquet système (pas de sudo)
#   --skip-flatpak   n'installe pas MakeMKV depuis Flathub
#   --help           affiche cette aide

set -euo pipefail

REPO_URL="https://github.com/loganxwolverine/intact-bitstream.git"
APP_ID="io.github.loganxwolverine.IntactBitstream"
PREFIX="${INTACT_BITSTREAM_PREFIX:-$HOME/.local}"
DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
SRC_DIR="$DATA_HOME/intact-bitstream/src"
MIN_RUST="1.92"
MIN_GTK="4.14"
MIN_ADW="1.5"
MAKEMKV_FLATPAK="com.makemkv.MakeMKV"
MKVTOOLNIX_FLATPAK="org.bunkus.mkvtoolnix-gui"
FLATHUB="https://dl.flathub.org/repo/flathub.flatpakrepo"

LAUNCH=1 SYSTEM=1 FLATPAK=1 UNINSTALL=0 PURGE=0 RELOGIN=0
for arg in "$@"; do
    case "$arg" in
        --uninstall) UNINSTALL=1 ;;
        --purge) PURGE=1 ;;
        --no-launch) LAUNCH=0 ;;
        --skip-system) SYSTEM=0 ;;
        --skip-flatpak) FLATPAK=0 ;;
        -h | --help) sed -n '2,/^set -euo/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "Option inconnue : $arg (voir --help)" >&2; exit 2 ;;
    esac
done

# --- Affichage ----------------------------------------------------------------
if [ -t 1 ]; then
    B=$'\e[1m' G=$'\e[32m' Y=$'\e[33m' R=$'\e[31m' C=$'\e[36m' N=$'\e[0m'
else
    B="" G="" Y="" R="" C="" N=""
fi
step() { printf '\n%s==> %s%s\n' "$B$C" "$*" "$N"; }
ok()   { printf '%s  ✔ %s%s\n' "$G" "$*" "$N"; }
warn() { printf '%s  ⚠ %s%s\n' "$Y" "$*" "$N" >&2; }
die()  { printf '\n%s✖ %s%s\n' "$R$B" "$*" "$N" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }
# Vrai si la version $1 est >= $2.
version_ge() { [ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -n1)" = "$2" ]; }

# Ancien nom de l'application (« Carbon Copy ») : retire l'ancienne entrée de
# menu et reprend les sources déjà téléchargées.
remove_legacy() {
    rm -f "$PREFIX/bin/carbon-copy" \
          "$DATA_HOME/applications/io.github.carboncopy.CarbonCopy.desktop" \
          "$DATA_HOME/icons/hicolor/scalable/apps/io.github.carboncopy.CarbonCopy.svg"
}

# --- Désinstallation ------------------------------------------------------------
if [ "$UNINSTALL" = 1 ]; then
    step "Désinstallation d'Intact Bitstream"
    remove_legacy
    rm -f "$PREFIX/bin/intact-bitstream" \
          "$DATA_HOME/applications/$APP_ID.desktop" \
          "$DATA_HOME/icons/hicolor/scalable/apps/$APP_ID.svg"
    if [ "$PURGE" = 1 ]; then
        rm -rf "$DATA_HOME/intact-bitstream" "${XDG_CONFIG_HOME:-$HOME/.config}/intact-bitstream"
        ok "Application, sources et réglages supprimés."
    else
        ok "Application supprimée (réglages conservés dans ~/.config/intact-bitstream)."
    fi
    echo "  MakeMKV (Flatpak) est conservé. Pour le retirer : flatpak uninstall --user $MAKEMKV_FLATPAK"
    exit 0
fi

[ "$(uname -s)" = Linux ] || die "Intact Bitstream fonctionne uniquement sous Linux."
[ -r /etc/os-release ] || die "Distribution inconnue (/etc/os-release absent)."
# shellcheck disable=SC1091
. /etc/os-release
DISTRO="${PRETTY_NAME:-$ID}"

# --- Gestionnaire de paquets ------------------------------------------------------
FAMILY=""
for id in ${ID:-} ${ID_LIKE:-}; do
    case "$id" in
        fedora | rhel | centos) FAMILY=dnf ;;
        debian | ubuntu) FAMILY=apt ;;
        arch | archlinux) FAMILY=pacman ;;
        suse | opensuse | opensuse-tumbleweed | opensuse-leap) FAMILY=zypper ;;
    esac
    [ -n "$FAMILY" ] && break
done

case "$FAMILY" in
    dnf) PKGS=(gcc pkgconf-pkg-config gtk4-devel libadwaita-devel flatpak mkvtoolnix util-linux git curl) ;;
    apt) PKGS=(build-essential pkg-config libgtk-4-dev libadwaita-1-dev flatpak mkvtoolnix eject git curl ca-certificates) ;;
    pacman) PKGS=(base-devel pkgconf gtk4 libadwaita flatpak mkvtoolnix-cli util-linux git curl) ;;
    zypper) PKGS=(gcc pkgconf-pkg-config gtk4-devel libadwaita-devel flatpak mkvtoolnix util-linux git curl) ;;
    *) PKGS=() ;;
esac

printf '%sIntact Bitstream%s — installation automatique sur %s\n' "$B" "$N" "$DISTRO"

# --- Droits administrateur (une seule demande) --------------------------------------
SUDO=""
if [ "$SYSTEM" = 1 ]; then
    if [ "$(id -u)" -ne 0 ]; then
        have sudo || die "La commande « sudo » est introuvable. Installez les paquets suivants en administrateur, puis relancez avec --skip-system : ${PKGS[*]}"
        SUDO="sudo"
        step "Droits administrateur"
        echo "  Votre mot de passe est demandé une seule fois (paquets système et accès au lecteur)."
        sudo -v </dev/tty || die "Authentification refusée."
        # Garde la session sudo active pendant la compilation (qui peut être longue).
        ( while kill -0 $$ 2>/dev/null; do sudo -n true 2>/dev/null; sleep 50; done ) &
    fi
fi

# --- 1. Paquets système ----------------------------------------------------------
if [ "$SYSTEM" = 1 ]; then
    step "1/6 Paquets système (${FAMILY:-?})"
    case "$FAMILY" in
        dnf)
            $SUDO dnf install -y "${PKGS[@]}" ;;
        apt)
            $SUDO env DEBIAN_FRONTEND=noninteractive apt-get update -q
            $SUDO env DEBIAN_FRONTEND=noninteractive apt-get install -y -q --no-install-recommends "${PKGS[@]}" ;;
        pacman)
            # Arch déconseille les mises à jour partielles : -Syu met tout le système à jour.
            $SUDO pacman -Syu --needed --noconfirm "${PKGS[@]}" ;;
        zypper)
            $SUDO zypper --non-interactive refresh
            $SUDO zypper --non-interactive install --no-recommends "${PKGS[@]}" ;;
        *)
            die "Distribution « $DISTRO » non reconnue. Installez à la main : GTK ≥ $MIN_GTK et libadwaita ≥ $MIN_ADW (paquets de développement), pkg-config, un compilateur C, Flatpak, MKVToolNix, git et curl ; puis relancez avec --skip-system." ;;
    esac
    ok "Paquets installés."
else
    step "1/6 Paquets système : ignorés (--skip-system)"
fi

# --- Vérification des bibliothèques graphiques ----------------------------------------
have pkg-config || die "pkg-config est introuvable."
GTK_V=$(pkg-config --modversion gtk4 2>/dev/null || echo 0)
ADW_V=$(pkg-config --modversion libadwaita-1 2>/dev/null || echo 0)
if ! version_ge "$GTK_V" "$MIN_GTK" || ! version_ge "$ADW_V" "$MIN_ADW"; then
    die "$DISTRO fournit GTK $GTK_V et libadwaita $ADW_V ; Intact Bitstream exige GTK ≥ $MIN_GTK et libadwaita ≥ $MIN_ADW.
   Distributions compatibles : Fedora 40+, Ubuntu 24.04+, Debian 13+, Arch, openSUSE Tumbleweed / Leap 16."
fi
ok "GTK $GTK_V, libadwaita $ADW_V"

# --- 2. Rust -------------------------------------------------------------------------
step "2/6 Compilateur Rust (≥ $MIN_RUST)"
export PATH="$HOME/.cargo/bin:$PATH"
RUST_V=$(rustc --version 2>/dev/null | awk '{print $2}' || true)
if [ -n "$RUST_V" ] && version_ge "$RUST_V" "$MIN_RUST" && have cargo; then
    ok "Rust $RUST_V déjà présent."
else
    [ -n "$RUST_V" ] && echo "  Rust $RUST_V est trop ancien : installation de rustup dans ~/.cargo (sans sudo)."
    if have rustup; then
        rustup toolchain install stable --profile minimal
        rustup default stable
    else
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path
    fi
    RUST_V=$(rustc --version | awk '{print $2}')
    version_ge "$RUST_V" "$MIN_RUST" || die "Impossible d'obtenir Rust ≥ $MIN_RUST (version actuelle : $RUST_V)."
    ok "Rust $RUST_V"
fi

# --- 3. Sources, compilation, installation ------------------------------------------------
step "3/6 Compilation d'Intact Bitstream"
if [ -d "$DATA_HOME/carbon-copy" ] && [ ! -e "$DATA_HOME/intact-bitstream" ]; then
    mv "$DATA_HOME/carbon-copy" "$DATA_HOME/intact-bitstream"
fi
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" 2>/dev/null && pwd || echo "")
if [ -n "$SCRIPT_DIR" ] && grep -qs '^name = "intact-bitstream"' "$SCRIPT_DIR/Cargo.toml"; then
    SRC="$SCRIPT_DIR"
    echo "  Sources locales : $SRC"
else
    SRC="$SRC_DIR"
    if [ -d "$SRC/.git" ]; then
        git -C "$SRC" pull --ff-only -q
        echo "  Sources mises à jour : $SRC"
    else
        mkdir -p "$(dirname "$SRC")"
        git clone --depth 1 -q "$REPO_URL" "$SRC"
        echo "  Sources téléchargées : $SRC"
    fi
fi
# Compilation : environ 1,5 Go de fichiers intermédiaires (plus Rust s'il vient d'être installé).
FREE_MB=$(df --output=avail -m "$SRC" | tail -n1)
if [ "${FREE_MB:-0}" -lt 2000 ]; then
    die "Espace disque insuffisant pour compiler : $((FREE_MB / 1024)) Go libres dans $SRC, il en faut au moins 2. Libérez de la place puis relancez."
fi
( cd "$SRC" && cargo build --release --locked )
install -Dm755 "$SRC/target/release/intact-bitstream" "$PREFIX/bin/intact-bitstream"
# Chemin absolu dans le lanceur : ~/.local/bin n'est pas toujours dans le PATH
# de la session graphique.
DESKTOP_TMP=$(mktemp)
sed "s|^Exec=.*|Exec=$PREFIX/bin/intact-bitstream|" "$SRC/data/$APP_ID.desktop" > "$DESKTOP_TMP"
install -Dm644 "$DESKTOP_TMP" "$DATA_HOME/applications/$APP_ID.desktop"
rm -f "$DESKTOP_TMP"
install -Dm644 "$SRC/data/icons/hicolor/scalable/apps/$APP_ID.svg" "$DATA_HOME/icons/hicolor/scalable/apps/$APP_ID.svg"
have update-desktop-database && update-desktop-database -q "$DATA_HOME/applications" 2>/dev/null || true
have gtk-update-icon-cache && gtk-update-icon-cache -q -t "$DATA_HOME/icons/hicolor" 2>/dev/null || true
remove_legacy
ok "Installée dans $PREFIX/bin/intact-bitstream (menu des applications + icône)."
# Sources téléchargées par le script : les fichiers de compilation ne servent plus.
if [ "$SRC" = "$SRC_DIR" ]; then
    ( cd "$SRC" && cargo clean -q ) && ok "Fichiers de compilation supprimés (place libérée)."
fi

# --- 4. MakeMKV et MKVToolNix --------------------------------------------------------------
step "4/6 MakeMKV (Flathub, mode utilisateur)"
if [ "$FLATPAK" = 0 ]; then
    echo "  Ignoré (--skip-flatpak) : l'application l'installera à son premier lancement."
elif ! have flatpak; then
    warn "Flatpak est absent : l'application ne pourra pas installer MakeMKV."
else
    flatpak remote-add --user --if-not-exists flathub "$FLATHUB"
    if flatpak info --user "$MAKEMKV_FLATPAK" >/dev/null 2>&1; then
        flatpak update --user -y --noninteractive "$MAKEMKV_FLATPAK" || warn "Mise à jour de MakeMKV impossible (hors-ligne ?)."
    else
        flatpak install --user -y --noninteractive flathub "$MAKEMKV_FLATPAK"
    fi
    ok "MakeMKV prêt. La clé bêta est récupérée automatiquement par l'application."
    if ! have mkvmerge; then
        flatpak install --user -y --noninteractive flathub "$MKVTOOLNIX_FLATPAK"
        ok "MKVToolNix installé depuis Flathub."
    fi
fi
have mkvmerge && ok "mkvmerge : $(command -v mkvmerge)"

# --- 5. Accès au lecteur ----------------------------------------------------------------
step "5/6 Accès au lecteur optique"
if [ "$SYSTEM" = 1 ]; then
    # Module « sg » : indispensable aux lecteurs Blu-ray, chargé à chaque démarrage.
    if [ ! -e /etc/modules-load.d/sg.conf ]; then
        { $SUDO mkdir -p /etc/modules-load.d && echo sg | $SUDO tee /etc/modules-load.d/sg.conf >/dev/null; } \
            || warn "Impossible d'écrire /etc/modules-load.d/sg.conf"
    fi
    if ! ls /dev/sg* >/dev/null 2>&1; then
        $SUDO modprobe sg 2>/dev/null || warn "Module « sg » non chargé (il le sera au prochain démarrage)."
    fi
fi
DRIVES=$(ls /dev/sr* 2>/dev/null || true)
if [ -z "$DRIVES" ]; then
    warn "Aucun lecteur optique détecté pour l'instant (branchez-le, l'application le verra)."
else
    for dev in $DRIVES; do
        if [ -r "$dev" ]; then
            ok "$dev accessible."
        elif [ "$SYSTEM" = 1 ]; then
            grp=$(stat -c %G "$dev")
            $SUDO usermod -aG "$grp" "$(id -un)"
            RELOGIN=1
            ok "Ajouté au groupe « $grp » pour accéder à $dev."
        else
            warn "$dev n'est pas lisible : ajoutez-vous au groupe « $(stat -c %G "$dev") »."
        fi
    done
fi

# --- 6. Lancement ------------------------------------------------------------------------
step "6/6 Terminé"
ok "Intact Bitstream est installé. Retrouvez-le dans le menu des applications."
if [ "$RELOGIN" = 1 ]; then
    warn "Fermez puis rouvrez votre session pour que l'accès au lecteur soit pris en compte."
    LAUNCH=0
fi
if [ "$LAUNCH" = 1 ] && { [ -n "${WAYLAND_DISPLAY:-}" ] || [ -n "${DISPLAY:-}" ]; }; then
    echo "  Lancement…"
    setsid "$PREFIX/bin/intact-bitstream" >/dev/null 2>&1 < /dev/null &
fi
