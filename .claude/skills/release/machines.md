# Release Build Machines

Single source of truth for all build targets and machine details.

## Local Machine (localhost)

No SSH required — commands run directly.

### Linux musl (x86_64)
```bash
cargo build --release --target x86_64-unknown-linux-musl --no-default-features --features rustls-backend
```
- Binary: `target/x86_64-unknown-linux-musl/release/clay`
- Release asset name: `clay-linux-x86_64-musl`

### Linux GUI+audio (x86_64, non-static)
```bash
cargo build --release --features webview-gui,native-audio
cp target/release/clay /tmp/clay-linux-x86_64-gui
```
- Binary: `target/release/clay` → copy to `/tmp/clay-linux-x86_64-gui` (avoids filename collision with musl build during upload)
- Release asset name: `clay-linux-x86_64-gui`
- Non-static (links glibc dynamically), includes WebView GUI and audio support

### Windows (pre-built by user)

The Windows binary is built natively on a Windows machine with MSVC (not cross-compiled).
The user places `clay.exe` in the project root directory before running `/release`.

```cmd
set RUSTFLAGS=-C target-feature=+crt-static
cargo build --release --features webview-gui,native-audio
```

- Binary: `clay.exe` (project root)
- Release asset name: `clay-windows-x86_64.exe`
- Static CRT linking (`+crt-static`) eliminates vcruntime140.dll dependency

### Android APK
```bash
cd android && ./gradlew assembleRelease
```
- Unsigned APK: `android/app/build/outputs/apk/release/app-release-unsigned.apk`
- After signing, the release asset name: `clay-android.apk`

**APK Signing:**
```bash
# Align
zipalign -v -p 4 android/app/build/outputs/apk/release/app-release-unsigned.apk android/clay-android-aligned.apk
# Sign
~/Android/Sdk/build-tools/35.0.0/apksigner sign --ks android/clay-release.keystore --ks-pass file:$HOME/.clay-keystore-pass --out android/clay-android.apk android/clay-android-aligned.apk
```

## Mac (192.168.2.12)

- User: `user`
- SSH port: `22`
- Path: `~/clay`
- SSH command: `ssh user@192.168.2.12`

### macOS universal binary (x86_64 + aarch64)
```bash
cd ~/clay
git pull
cargo build --release --target x86_64-apple-darwin --features webview-gui,native-audio
cargo build --release --target aarch64-apple-darwin --features webview-gui,native-audio
lipo -create \
    target/x86_64-apple-darwin/release/clay \
    target/aarch64-apple-darwin/release/clay \
    -output clay-macos-universal
```
- Binary: `~/clay/clay-macos-universal`
- Release asset name: `clay-macos-universal`
- SCP back: `scp user@192.168.2.12:~/clay/clay-macos-universal /tmp/clay-macos-universal`

## Termux (192.168.2.50)

- User: `adrick`
- SSH port: `8022`
- Path: `~/clay`
- SSH command: `ssh -p 8022 adrick@192.168.2.50`

### Termux aarch64 binary (with GUI, no audio)
```bash
cd ~/clay
git pull
./patches/apply-patches.sh
PKG_CONFIG_PATH=/data/data/com.termux/files/usr/lib/pkgconfig RUSTFLAGS="-L /system/lib64 -C link-arg=-Wl,-rpath,/system/lib64" cargo build --release --no-default-features --features rustls-backend,webview-gui
patchelf --set-rpath '/system/lib64:/data/data/com.termux/files/usr/lib' target/release/clay
```
- Binary: `~/clay/target/release/clay`
- Release asset name: `clay-termux-aarch64`
- SCP back: `scp -P 8022 adrick@192.168.2.50:~/clay/target/release/clay /tmp/clay-termux-aarch64`

## Linux Test (192.168.2.6) — VERIFICATION ONLY

- User: `adrick`
- SSH port: `22`
- Path: `~/clay.build`
- SSH command: `ssh adrick@192.168.2.6`

### Build verification (NOT uploaded)
```bash
cd ~/clay.build
git pull
. ~/.cargo/env && cargo build --release --no-default-features --features rustls-backend
```
- This build is only to verify the code compiles on this machine
- The binary is NOT included in the release assets
- Report pass/fail in the summary
