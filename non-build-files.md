# Files not needed to build Clay

As of commit `4ef9ed8`, 2026-10-02, after the deletions listed under "Deleted".

The repo tracks 310 files. Building Clay needs 91 of them: `Cargo.toml`, `Cargo.lock`,
`build.rs`, `clay2.png`, `clay_icon.png` (the Windows icon) and 86 of the 91 files in `src/`.
That was checked against the lists of files the compiler read during the release builds.
Another 25 are markdown help files: `README.md` and the 24 in `docs/markdown/`. The remaining
194 are below, starting with the likeliest leftovers.

## Leftovers nothing uses (3)

- `README.test.md`: an older draft of the README.
- `clay-documentation.pdf`: the manual as a PDF, last updated in January. Nothing links to it.
- `crypt.tf`: a sample TinyFugue script. The tests that use it carry their own copy.

## Deleted

Removed on 2026-10-02:

- `.dockerignore`: left over from the Docker builds. There's no Dockerfile any more.
- `patches/glutin-0.30.10.patch`, `patches/glutin-winit-0.3.0.patch`,
  `patches/winit-0.28.7.patch`: from the old GUI. `apply-patches.sh` only applies the tao and
  wry patches.

## Build scripts (11)

- **End users:** `build.sh`, the current script for building Clay yourself. Nothing else in
  the repo refers to it, but it is in use.
- **Android:** `android.sh`, `build-android-aarch64.sh`
- **Termux:** `build-termux-aarch64.sh`, `build-termux-armv7.sh`,
  `build-termux-aarch64-gui.sh`, `tools/termux-sysroot.py`, `patches/apply-patches.sh`,
  `patches/tao-0.34.5.patch`, `patches/wry-0.48.1.patch`
- **Drobo 5N:** `build-drobo5n.sh`

## The Android app (46)

Everything in `android/`: the Gradle project (Java sources, resources, build files, Gradle
wrapper), plus three files that aren't source:

- `android/clay-android.apk` (15 MB) and `android/clay-android.apk.idsig` are the built app.
  Every release commits a new copy, which adds about 15 MB to the repo's history each time.
- `android/clay-release.keystore` is the app's signing key. Its password isn't in the repo, but
  the key itself is public.

## Tests (115)

- `tests/tf/cases/`: 52 `.tf` scripts and their 52 `.expected` outputs
- `tests/tf/xfail.txt`, `tests/tf/README.md`, `tests/gui_startup.rs`
- `src/tests.rs`, `src/testharness.rs`, `src/chat/test_support.rs`, `src/tf/script_tests.rs`:
  compiled only by `cargo test`
- `src/bin/clay-test-server.rs`: a fake MUD server. `cargo build` builds it as a second program
  because `Cargo.toml` declares it, but Clay doesn't need it. Deleting it means removing that
  entry from `Cargo.toml` too.
- `tools/tf-oracle.sh`: generates the `.expected` files from real tf
- `reference/fake-mud-server.py`, `reference/ws-verify-client.py`: manual test tools

## Markdown, but not help (14)

- `CLAUDE.md`, `.claude/skills/release/SKILL.md`, `.claude/skills/release/machines.md`:
  instructions for Claude Code
- `PROTOCOL-ROADMAP.md`, `SECURITY-ROADMAP.md`, `EMOJI-PICKER-ROADMAP.md`,
  `TINYFUGUE-COMPAT.md`: design records
- `reference/commands.md`, `reference/features.md`, `reference/networking.md`,
  `reference/tf-engine.md`: developer reference
- `websockets.readme`: the WebSocket protocol spec
- `SECURITY-NOTES.md`, `PRIVACY.md`: written for users, but not part of the manual

## Repo housekeeping (5)

- `.gitignore`, `.gitattributes`, `LICENSE`
- `.cargo/audit.toml`: one security advisory for `cargo audit` to ignore
- `screenshot.png`: the README's picture

## Also noted

The manual itself has broken pictures: `03-quickstart.md` and `08-settings.md` show
screenshots from `docs/images/`, and none of those images are in the repo, so they don't appear
on GitHub.

## Complete list (194)

```
android/app/build.gradle
android/app/proguard-rules.pro
android/app/src/main/AndroidManifest.xml
android/app/src/main/java/com/clay/mudclient/CertPinning.java
android/app/src/main/java/com/clay/mudclient/ClayForegroundService.java
android/app/src/main/java/com/clay/mudclient/ClaySession.java
android/app/src/main/java/com/clay/mudclient/LocalServerManager.java
android/app/src/main/java/com/clay/mudclient/MainActivity.java
android/app/src/main/java/com/clay/mudclient/NativeWebSocket.java
android/app/src/main/java/com/clay/mudclient/SshProxyManager.java
android/app/src/main/res/drawable/auth_key_bg.xml
android/app/src/main/res/drawable/ic_launcher_background.xml
android/app/src/main/res/drawable/ic_launcher_foreground.xml
android/app/src/main/res/drawable/ic_notification.xml
android/app/src/main/res/layout/activity_main.xml
android/app/src/main/res/mipmap-anydpi-v26/ic_launcher_round.xml
android/app/src/main/res/mipmap-anydpi-v26/ic_launcher.xml
android/app/src/main/res/mipmap-hdpi/ic_launcher_foreground.png
android/app/src/main/res/mipmap-hdpi/ic_launcher.png
android/app/src/main/res/mipmap-hdpi/ic_launcher_round.png
android/app/src/main/res/mipmap-mdpi/ic_launcher_foreground.png
android/app/src/main/res/mipmap-mdpi/ic_launcher.png
android/app/src/main/res/mipmap-mdpi/ic_launcher_round.png
android/app/src/main/res/mipmap-xhdpi/ic_launcher_foreground.png
android/app/src/main/res/mipmap-xhdpi/ic_launcher.png
android/app/src/main/res/mipmap-xhdpi/ic_launcher_round.png
android/app/src/main/res/mipmap-xxhdpi/ic_launcher_foreground.png
android/app/src/main/res/mipmap-xxhdpi/ic_launcher.png
android/app/src/main/res/mipmap-xxhdpi/ic_launcher_round.png
android/app/src/main/res/mipmap-xxxhdpi/ic_launcher_foreground.png
android/app/src/main/res/mipmap-xxxhdpi/ic_launcher.png
android/app/src/main/res/mipmap-xxxhdpi/ic_launcher_round.png
android/app/src/main/res/values/strings.xml
android/app/src/main/res/values/themes.xml
android/app/src/main/res/xml/file_paths.xml
android/app/src/main/res/xml/network_security_config.xml
android/build.gradle
android/clay-android.apk
android/clay-android.apk.idsig
android/clay-release.keystore
android/.gitignore
android/gradle.properties
android/gradlew
android/gradle/wrapper/gradle-wrapper.jar
android/gradle/wrapper/gradle-wrapper.properties
android/settings.gradle
android.sh
build-android-aarch64.sh
build-drobo5n.sh
build.sh
build-termux-aarch64-gui.sh
build-termux-aarch64.sh
build-termux-armv7.sh
.cargo/audit.toml
CLAUDE.md
.claude/skills/release/machines.md
.claude/skills/release/SKILL.md
clay-documentation.pdf
crypt.tf
EMOJI-PICKER-ROADMAP.md
.gitattributes
.gitignore
LICENSE
patches/apply-patches.sh
patches/tao-0.34.5.patch
patches/wry-0.48.1.patch
PRIVACY.md
PROTOCOL-ROADMAP.md
README.test.md
reference/commands.md
reference/fake-mud-server.py
reference/features.md
reference/networking.md
reference/tf-engine.md
reference/ws-verify-client.py
screenshot.png
SECURITY-NOTES.md
SECURITY-ROADMAP.md
src/bin/clay-test-server.rs
src/chat/test_support.rs
src/testharness.rs
src/tests.rs
src/tf/script_tests.rs
tests/gui_startup.rs
tests/tf/cases/argspace.expected
tests/tf/cases/argspace.tf
tests/tf/cases/at_prefix.expected
tests/tf/cases/at_prefix.tf
tests/tf/cases/control_flow.expected
tests/tf/cases/control_flow.tf
tests/tf/cases/eval.expected
tests/tf/cases/eval.tf
tests/tf/cases/for_syntax.expected
tests/tf/cases/for_syntax.tf
tests/tf/cases/functions.expected
tests/tf/cases/functions.tf
tests/tf/cases/help.expected
tests/tf/cases/help.tf
tests/tf/cases/hooks.expected
tests/tf/cases/hooks.tf
tests/tf/cases/if_command.expected
tests/tf/cases/if_command.tf
tests/tf/cases/lib_alias.expected
tests/tf/cases/lib_alias.tf
tests/tf/cases/lib_at.expected
tests/tf/cases/lib_at.tf
tests/tf/cases/lib_color.expected
tests/tf/cases/lib_color.tf
tests/tf/cases/lib_complete.expected
tests/tf/cases/lib_complete.tf
tests/tf/cases/lib_cylon.expected
tests/tf/cases/lib_cylon.tf
tests/tf/cases/lib_factoral.expected
tests/tf/cases/lib_factoral.tf
tests/tf/cases/lib_grep.expected
tests/tf/cases/lib_grep.tf
tests/tf/cases/lib_hanoi.expected
tests/tf/cases/lib_hanoi.tf
tests/tf/cases/lib_kbbind.expected
tests/tf/cases/lib_kbbind.tf
tests/tf/cases/lib_kbfunc.expected
tests/tf/cases/lib_kbfunc.tf
tests/tf/cases/lib_kbregion.expected
tests/tf/cases/lib_kbregion.tf
tests/tf/cases/lib_kbstack.expected
tests/tf/cases/lib_kbstack.tf
tests/tf/cases/lib_lisp.expected
tests/tf/cases/lib_lisp.tf
tests/tf/cases/lib_map.expected
tests/tf/cases/lib_map.tf
tests/tf/cases/lib_quoter.expected
tests/tf/cases/lib_quoter.tf
tests/tf/cases/lib_self.expected
tests/tf/cases/lib_self.tf
tests/tf/cases/lib_spedwalk.expected
tests/tf/cases/lib_spedwalk.tf
tests/tf/cases/lib_stack-q.expected
tests/tf/cases/lib_stack-q.tf
tests/tf/cases/lib_testcolor.expected
tests/tf/cases/lib_testcolor.tf
tests/tf/cases/lib_textencode.expected
tests/tf/cases/lib_textencode.tf
tests/tf/cases/lib_textutil.expected
tests/tf/cases/lib_textutil.tf
tests/tf/cases/lib_tick.expected
tests/tf/cases/lib_tick.tf
tests/tf/cases/lib_tintin.expected
tests/tf/cases/lib_tintin.tf
tests/tf/cases/lib_tools.expected
tests/tf/cases/lib_tools.tf
tests/tf/cases/lib_tr.expected
tests/tf/cases/lib_tr.tf
tests/tf/cases/lib_worldq.expected
tests/tf/cases/lib_worldq.tf
tests/tf/cases/macro_result.expected
tests/tf/cases/macro_result.tf
tests/tf/cases/macros.expected
tests/tf/cases/macros.tf
tests/tf/cases/mecho.expected
tests/tf/cases/mecho.tf
tests/tf/cases/pipes.expected
tests/tf/cases/pipes.tf
tests/tf/cases/positional.expected
tests/tf/cases/positional.tf
tests/tf/cases/purge_args.expected
tests/tf/cases/purge_args.tf
tests/tf/cases/quote.expected
tests/tf/cases/quote.tf
tests/tf/cases/special_vars.expected
tests/tf/cases/special_vars.tf
tests/tf/cases/status.expected
tests/tf/cases/status.tf
tests/tf/cases/stdlib_macros.expected
tests/tf/cases/stdlib_macros.tf
tests/tf/cases/strings.expected
tests/tf/cases/strings.tf
tests/tf/cases/time.expected
tests/tf/cases/time.tf
tests/tf/cases/toplevel_nosub.expected
tests/tf/cases/toplevel_nosub.tf
tests/tf/cases/trailing_space.expected
tests/tf/cases/trailing_space.tf
tests/tf/cases/trigger.expected
tests/tf/cases/trigger.tf
tests/tf/cases/truthiness.expected
tests/tf/cases/truthiness.tf
tests/tf/cases/undefn.expected
tests/tf/cases/undefn.tf
tests/tf/README.md
tests/tf/xfail.txt
TINYFUGUE-COMPAT.md
tools/termux-sysroot.py
tools/tf-oracle.sh
websockets.readme
```
