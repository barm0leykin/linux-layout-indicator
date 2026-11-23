#!/usr/bin/env bash
# Сборка .deb без внешних инструментов — только cargo и dpkg-deb.
#
# Использование:
#   packaging/build-deb.sh            # собрать пакет
#   packaging/build-deb.sh --no-build  # не пересобирать бинарник
set -euo pipefail

cd "$(dirname "$0")/.."

VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
ARCH=$(dpkg --print-architecture)
OUT=target/debian
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
chmod 755 "$STAGE"

if [ "${1:-}" != "--no-build" ]; then
    cargo build --release --locked
fi

install -Dm755 target/release/layout-indicator \
    "$STAGE/usr/bin/layout-indicator"
install -Dm644 packaging/layout-indicator.desktop \
    "$STAGE/usr/share/applications/layout-indicator.desktop"
install -Dm644 LICENSE "$STAGE/usr/share/doc/layout-indicator/copyright"
install -Dm644 README.md "$STAGE/usr/share/doc/layout-indicator/README.md"
install -Dm644 config.example.yaml \
    "$STAGE/usr/share/doc/layout-indicator/config.example.yaml"

# Иконки трея — в тему hicolor, чтобы Host находил их по IconName.
for size in 22 24 32 48; do
    install -Dm644 "assets/icon-${size}.png" \
        "$STAGE/usr/share/icons/hicolor/${size}x${size}/apps/layout-indicator.png"
done

mkdir -p "$STAGE/DEBIAN"
SIZE=$(du -sk "$STAGE/usr" | cut -f1)
cat > "$STAGE/DEBIAN/control" <<EOF
Package: layout-indicator
Version: ${VERSION}
Architecture: ${ARCH}
Maintainer: layout-indicator contributors
Installed-Size: ${SIZE}
Depends: libc6, libgcc-s1
Section: utils
Priority: optional
Description: Индикатор текущей раскладки клавиатуры у курсора
 Всплывающий бейдж EN/RU рядом с курсором при переключении раскладки.
 Работает на X11, не перехватывает ввод и не требует прав root.
EOF

mkdir -p "$OUT"
DEB="$OUT/layout-indicator_${VERSION}_${ARCH}.deb"
dpkg-deb --root-owner-group --build "$STAGE" "$DEB" >/dev/null
echo "готово: $DEB"
