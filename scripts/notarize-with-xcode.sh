#!/bin/bash
# Ký lại một .app bằng Developer ID do Apple quản lý trên đám mây (cloud-managed
# signing) rồi notarize + staple, dùng tài khoản đã đăng nhập trong Xcode.
# Không cần chứng thư Developer ID trong Keychain hay mật khẩu notarytool.
#
# Dùng: notarize-with-xcode.sh <đường-dẫn-.app> <team-id>
# App đầu vào phải đã được ký (không ad-hoc) kèm entitlements, vì Xcode giữ
# entitlements của chữ ký hiện có khi ký lại. App được thay tại chỗ.
set -euo pipefail

APP_PATH="${1:?Thiếu đường dẫn .app}"
TEAM_ID="${2:?Thiếu team ID}"
POLL_SECONDS=15
POLL_ATTEMPTS=60

INFO_PLIST="$APP_PATH/Contents/Info.plist"
read_plist() { /usr/libexec/PlistBuddy -c "Print $1" "$INFO_PLIST"; }
APP_NAME="$(basename "$APP_PATH" .app)"
BUNDLE_ID="$(read_plist CFBundleIdentifier)"
VERSION="$(read_plist CFBundleShortVersionString)"
BUILD_NUMBER="$(read_plist CFBundleVersion)"

WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/xhub-notarize.XXXXXX")"
trap 'rm -rf "$WORK_DIR"' EXIT
ARCHIVE="$WORK_DIR/$APP_NAME.xcarchive"

# xcodebuild -exportArchive chỉ nhận .xcarchive, nên dựng tay một archive
# chứa bundle đã build sẵn.
mkdir -p "$ARCHIVE/Products/Applications"
ditto "$APP_PATH" "$ARCHIVE/Products/Applications/$APP_NAME.app"
cat > "$ARCHIVE/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>ApplicationProperties</key><dict>
<key>ApplicationPath</key><string>Applications/$APP_NAME.app</string>
<key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
<key>CFBundleShortVersionString</key><string>$VERSION</string>
<key>CFBundleVersion</key><string>$BUILD_NUMBER</string>
<key>Team</key><string>$TEAM_ID</string>
</dict>
<key>ArchiveVersion</key><integer>2</integer>
<key>CreationDate</key><date>$(date -u +%Y-%m-%dT%H:%M:%SZ)</date>
<key>Name</key><string>$APP_NAME</string>
<key>SchemeName</key><string>$APP_NAME</string>
</dict></plist>
EOF

cat > "$WORK_DIR/ExportOptions.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>method</key><string>developer-id</string>
<key>teamID</key><string>$TEAM_ID</string>
<key>signingStyle</key><string>automatic</string>
<key>destination</key><string>upload</string>
</dict></plist>
EOF

echo "Signing with cloud Developer ID and uploading for notarization..."
if ! xcodebuild -exportArchive -archivePath "$ARCHIVE" \
    -exportOptionsPlist "$WORK_DIR/ExportOptions.plist" \
    -exportPath "$WORK_DIR/upload" -allowProvisioningUpdates \
    > "$WORK_DIR/export.log" 2>&1; then
    tail -20 "$WORK_DIR/export.log" >&2
    exit 1
fi

echo "Waiting for Apple notarization..."
NOTARIZED_DIR="$WORK_DIR/notarized"
for ((attempt = 1; attempt <= POLL_ATTEMPTS; attempt++)); do
    if xcodebuild -exportNotarizedApp -archivePath "$ARCHIVE" \
        -exportPath "$NOTARIZED_DIR" > "$WORK_DIR/notarized.log" 2>&1; then
        break
    fi
    if ((attempt == POLL_ATTEMPTS)); then
        echo "Notarization chưa xong sau $((POLL_SECONDS * POLL_ATTEMPTS))s." >&2
        tail -20 "$WORK_DIR/notarized.log" >&2
        exit 1
    fi
    sleep "$POLL_SECONDS"
done

xcrun stapler validate "$NOTARIZED_DIR/$APP_NAME.app" >/dev/null
spctl --assess --type execute "$NOTARIZED_DIR/$APP_NAME.app"

rm -rf "$APP_PATH"
ditto "$NOTARIZED_DIR/$APP_NAME.app" "$APP_PATH"
echo "Notarized: $(codesign -dvv "$APP_PATH" 2>&1 | grep -m1 '^Authority=')"
