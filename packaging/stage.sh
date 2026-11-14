#!/usr/bin/env bash
# Copie les fichiers de Carbon Copy dans une arborescence /usr sous DESTDIR.
# Usage : packaging/stage.sh <DESTDIR> [binaire]
set -euo pipefail
DEST=${1:?DESTDIR}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
BIN=${2:-$ROOT/target/release/carbon-copy}
ID=io.github.carboncopy.CarbonCopy

install -Dm755 "$BIN" "$DEST/usr/bin/carbon-copy"
install -Dm644 "$ROOT/data/$ID.desktop" "$DEST/usr/share/applications/$ID.desktop"
install -Dm644 "$ROOT/data/$ID.metainfo.xml" "$DEST/usr/share/metainfo/$ID.metainfo.xml"
install -Dm644 "$ROOT/data/icons/hicolor/scalable/apps/$ID.svg" "$DEST/usr/share/icons/hicolor/scalable/apps/$ID.svg"
install -Dm644 "$ROOT/packaging/carbon-copy.rules" "$DEST/usr/lib/udev/rules.d/70-carbon-copy.rules"
install -d "$DEST/usr/lib/modules-load.d"
echo sg > "$DEST/usr/lib/modules-load.d/carbon-copy.conf"
chmod 644 "$DEST/usr/lib/modules-load.d/carbon-copy.conf"
install -Dm644 "$ROOT/LICENSE" "$DEST/usr/share/licenses/carbon-copy/LICENSE"
