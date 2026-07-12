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

**Note:** Windows is now built remotely on the Windows VM (see below). This section is kept
for fallback: if the VM is unavailable and the user hand-builds `clay.exe`, place it in the
project root before running `/release`.

```cmd
set RUSTFLAGS=-C target-feature=+crt-static
cargo build --release --features webview-gui,native-audio
```

- Binary: `clay.exe` (project root, fallback only)
- Release asset name: `clay-windows-x86_64.exe`
- Static CRT linking (`+crt-static`) eliminates vcruntime140.dll dependency

### Termux armv7 binary (cross-compiled, no GUI)

Cross-compiled here on localhost via the Android NDK — there is no 32-bit device available, so
unlike the aarch64 Termux binaries below (built natively on-device), this one targets
`armv7-linux-androideabi` using the NDK's clang. No-GUI only (`rustls-backend`), so no
tao/wry/X11 patches or libraries are needed.

**One-time setup:**
- Android NDK r26d unpacked at `~/Android/Sdk/ndk/26.3.11579264` (no `sdkmanager`/cmdline-tools
  needed — downloaded directly from `https://dl.google.com/android/repository/android-ndk-r26d-linux.zip`)
- `rustup target add armv7-linux-androideabi`
- `patchelf` on PATH — installed to `~/.local/bin` from the prebuilt static release
  (`https://github.com/NixOS/patchelf/releases/download/0.18.0/patchelf-0.18.0-x86_64.tar.gz`);
  no root/apt required

**Build:**
```bash
./build-termux-armv7.sh
```
This sets `CC_armv7_linux_androideabi`/`CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_LINKER` etc. to the
NDK's `armv7a-linux-androideabi24-clang`, runs `cargo build --release --target armv7-linux-androideabi
--no-default-features --features rustls-backend`, then `patchelf --set-rpath` (mirroring the aarch64
convention but with `/system/lib` instead of `/system/lib64`), and copies the result out.
- Binary: `target/armv7-linux-androideabi/release/clay` → copied to `/tmp/clay-termux-armv7-32bit-nogui`
- Release asset name: `clay-termux-armv7-32bit-nogui`
- Verified: builds clean, produces a valid `ELF32 ARM EABI5` binary, and has been confirmed to run
  on the aarch64 Termux phone (192.168.2.50) via its 32-bit userspace compatibility layer.

### Android APK
```bash
cd android && JAVA_HOME=/usr/lib/jvm/java-21-openjdk-amd64 ./gradlew assembleRelease
```
**Note:** The system default JDK (Java 25) is too new for Gradle 8.2 — always force Java 21 via `JAVA_HOME`.
- Unsigned APK: `android/app/build/outputs/apk/release/app-release-unsigned.apk`
- After signing, the release asset name: `clay-android.apk`

**APK Signing:**
```bash
# Align
zipalign -v -p 4 android/app/build/outputs/apk/release/app-release-unsigned.apk android/clay-android-aligned.apk
# Sign
~/Android/Sdk/build-tools/35.0.0/apksigner sign --ks android/clay-release.keystore --ks-pass file:$HOME/.clay-keystore-pass --out android/clay-android.apk android/clay-android-aligned.apk
```

## Windows VM (192.168.2.14) — VirtualBox guest on Linux host

- User: `adrick`
- SSH port: `22`
- Path: `C:\Users\adrick\clay`
- SSH command: `ssh adrick@192.168.2.14`
- **Lifecycle:** VM is kept powered off when not in use. `/release` starts it before building and powers it off after (see SKILL.md Step 6b).
- Start: `VBoxManage startvm clay-win11 --type headless`
- Stop: `VBoxManage controlvm clay-win11 poweroff`

### Windows x86_64 binary (MSVC, GUI + audio)
```cmd
cd clay && git pull && set RUSTFLAGS=-C target-feature=+crt-static && cargo build --release --features webview-gui,native-audio
```
- Binary: `target\release\clay.exe`
- Release asset name: `clay-windows-x86_64.exe`
- SCP back: `scp adrick@192.168.2.14:clay/target/release/clay.exe /tmp/clay-windows-x86_64.exe`
- Static CRT (`+crt-static`) eliminates vcruntime140.dll dependency

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

### Termux aarch64 binary (no GUI, reduced library needs)

Built after the GUI binary, once `Cargo.toml`/`Cargo.lock` have been restored to their git state (no tao/wry patches applied) — this build needs no X11/webview libraries at all.

```bash
cd ~/clay
cargo build --release --no-default-features --features rustls-backend
```
- Binary: `~/clay/target/release/clay`
- Release asset name: `clay-termux-aarch64-nogui`
- SCP back: `scp -P 8022 adrick@192.168.2.50:~/clay/target/release/clay /tmp/clay-termux-aarch64-nogui`

## Linux Test (192.168.2.6) — VERIFICATION ONLY

- User: `adrick`
- SSH port: `22`
- Path: `~/clay.build`
- SSH command: `ssh adrick@192.168.2.6`

### Build verification (NOT uploaded)
```bash
cd ~/clay.build
git pull
. ~/.cargo/env && cargo build --release --features webview-gui,native-audio
```
- This build is only to verify the code compiles on this machine (including GUI)
- The binary is NOT included in the release assets
- Report pass/fail in the summary
