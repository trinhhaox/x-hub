# Xhub

Ứng dụng macOS quản lý iPhone và điện thoại Android: danh sách thiết bị qua USB và Wi-Fi, xem màn hình ngay trong app (điều khiển được với Android), cài app (.apk / .ipa), chụp màn hình, bật kết nối không dây.

- Trang chủ: https://xhub.xox.vn
- Yêu cầu: máy Mac chip Apple; Android cần `brew install scrcpy android-platform-tools`; iPhone cần Xcode.

## Phát triển

```bash
pnpm install
pnpm tauri dev
```

Phát hành (ký Developer ID + notarize, tải lên GitHub release):

```bash
CODE_SIGN_IDENTITY=<Apple Development hash> XCODE_NOTARIZE_TEAM=<team id> scripts/release-notarized.sh --upload
```
