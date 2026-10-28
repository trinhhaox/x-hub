#!/bin/bash
# Build Xhub, ký Developer ID (cloud-managed qua Xcode), notarize, rồi tạo DMG,
# gói cập nhật đã ký (tar.gz + .sig) và latest.json cho updater.
# Dùng: CODE_SIGN_IDENTITY=<id Apple Development> XCODE_NOTARIZE_TEAM=<team> \
#       scripts/release-notarized.sh [--upload]
# --upload: tải các file lên GitHub release v<version> (tạo mới nếu chưa có).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
IDENTITY="${CODE_SIGN_IDENTITY:?Thiếu CODE_SIGN_IDENTITY}"
TEAM="${XCODE_NOTARIZE_TEAM:?Thiếu XCODE_NOTARIZE_TEAM}"
UPDATER_KEY="${TAURI_KEY_PATH:-$HOME/.tauri/xhub.key}"
REPO="trinhhaox/x-hub"
VERSION="$(node -p "require('./package.json').version")"
ARCH="aarch64"
ENTITLEMENTS="src-tauri/Entitlements.plist"
APP="src-tauri/target/release/bundle/macos/Xhub.app"
OUT="src-tauri/target/release/notarized"

# Khoá ký updater: tauri build cần nó (createUpdaterArtifacts) và dùng để ký gói cập nhật
export TAURI_SIGNING_PRIVATE_KEY="$(cat "$UPDATER_KEY")"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""

echo "==> Build Xhub $VERSION"
pnpm tauri build --bundles app

# Ký trong ra ngoài, bật hardened runtime; Xcode giữ entitlements khi ký lại bằng Developer ID
echo "==> Ký tạm bằng $IDENTITY"
for target in "$APP/Contents/MacOS/ios-screen-capture" "$APP"; do
    codesign --force --options runtime --timestamp --entitlements "$ENTITLEMENTS" \
        --sign "$IDENTITY" "$target"
done
codesign --verify --deep --strict "$APP"

echo "==> Ký Developer ID + notarize"
scripts/notarize-with-xcode.sh "$APP" "$TEAM"

echo "==> Đóng gói"
rm -rf "$OUT" && mkdir -p "$OUT"
UPDATER="$OUT/Xhub_${ARCH}.app.tar.gz"
COPYFILE_DISABLE=1 tar -czf "$UPDATER" -C "$(dirname "$APP")" Xhub.app
pnpm tauri signer sign "$UPDATER" >/dev/null

STAGING="$(mktemp -d)"
trap 'rm -rf "$STAGING"' EXIT
ditto "$APP" "$STAGING/Xhub.app"
ln -s /Applications "$STAGING/Applications"
hdiutil create -quiet -volname "Xhub" -srcfolder "$STAGING" -format UDZO "$OUT/Xhub_${VERSION}_${ARCH}.dmg"

NOTES="$(git log -1 --format=%s "v$VERSION" 2>/dev/null || echo "Xhub $VERSION")"
NOTES="${RELEASE_NOTES:-$NOTES}" VERSION="$VERSION" ARCH="$ARCH" REPO="$REPO" OUT="$OUT" python3 - <<'PY'
import datetime, json, os
out, version, arch, repo = os.environ["OUT"], os.environ["VERSION"], os.environ["ARCH"], os.environ["REPO"]
name = f"Xhub_{arch}.app.tar.gz"
signature = open(f"{out}/{name}.sig").read().strip()
url = f"https://github.com/{repo}/releases/download/v{version}/{name}"
entry = {"signature": signature, "url": url}
manifest = {
    "version": version,
    "notes": os.environ["NOTES"],
    "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.000Z"),
    "platforms": {f"darwin-{arch}": entry, f"darwin-{arch}-app": entry},
}
json.dump(manifest, open(f"{out}/latest.json", "w"), ensure_ascii=False, indent=2)
PY

spctl --assess --type execute "$APP" && echo "Gatekeeper: OK"
ls -la "$OUT"

if [[ "${1:-}" == "--upload" ]]; then
    echo "==> Tải lên release v$VERSION"
    if gh release view "v$VERSION" -R "$REPO" >/dev/null 2>&1; then
        gh release upload "v$VERSION" -R "$REPO" --clobber "$OUT"/*
    else
        gh release create "v$VERSION" -R "$REPO" --title "Xhub v$VERSION" --notes "$NOTES" "$OUT"/*
    fi
fi
