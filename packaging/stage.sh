#!/usr/bin/env bash
# Copie les fichiers d'Intact Bitstream dans une arborescence /usr sous DESTDIR.
# Usage : packaging/stage.sh <DESTDIR> [binaire]
set -euo pipefail
DEST=${1:?DESTDIR}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
BIN=${2:-$ROOT/target/release/intact-bitstream}
ID=io.github.loganxwolverine.IntactBitstream

install -Dm755 "$BIN" "$DEST/usr/bin/intact-bitstream"
install -Dm644 "$ROOT/data/$ID.desktop" "$DEST/usr/share/applications/$ID.desktop"
install -Dm644 "$ROOT/data/$ID.metainfo.xml" "$DEST/usr/share/metainfo/$ID.metainfo.xml"
install -Dm644 "$ROOT/data/icons/hicolor/scalable/apps/$ID.svg" "$DEST/usr/share/icons/hicolor/scalable/apps/$ID.svg"
install -Dm644 "$ROOT/packaging/intact-bitstream.rules" "$DEST/usr/lib/udev/rules.d/70-intact-bitstream.rules"
install -d "$DEST/usr/lib/modules-load.d"
echo sg > "$DEST/usr/lib/modules-load.d/intact-bitstream.conf"
chmod 644 "$DEST/usr/lib/modules-load.d/intact-bitstream.conf"
install -Dm644 "$ROOT/LICENSE" "$DEST/usr/share/licenses/intact-bitstream/LICENSE"
