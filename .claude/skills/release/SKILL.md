---
name: release
description: Automated multi-platform build and GitHub release for Clay. Builds on local (Linux musl, Windows, Android) and remote (macOS, Termux) machines, uploads to GitHub.
---

# /release — Automated Multi-Platform Build & GitHub Release

This skill automates building Clay on all target platforms and uploading release binaries to GitHub.

The user's arguments are available as: $ARGUMENTS

## Instructions

Follow these steps in order. You are the orchestrator — read build output, diagnose errors, fix code if needed, and retry. Do NOT ask the user any questions during the process — handle everything automatically. The only exception is if a build fails due to an environment issue that you cannot fix.

### Step 1: Determine Version and Options

Parse `$ARGUMENTS` for the version and any platform flags:

- **Version**: The first non-flag token. Prepend `v` if it doesn't start with `v`. Normalize (e.g., "1.0 beta" → "v1.0.0-beta"). If no version token, read from `Cargo.toml` line 3.
- **Platform flags**: `--android`, `--mac`, `--linux` — at most one may be specified. If present, the release is limited to that platform only (see "Platform-limited release" below).
- **Termux / Mac**: Reachability is auto-detected at Step 6 — do NOT ask the user about these. If a machine is unreachable, it is silently skipped.

#### Platform-limited release (`--android` / `--mac` / `--linux`)

When a platform flag is given:
- Only build the specified platform (skip all others).
- Only upload that platform's asset(s) to GitHub — do **not** delete or modify any other existing assets.
- Skip the Termux question.
- Still commit any pending work, do the version bump, commit, and push (Steps 3–4) so the binary matches the tagged version.

Platform → assets built and uploaded:

| Flag | Builds | GitHub assets touched |
|------|--------|-----------------------|
| `--android` | Android APK | `clay-android.apk` |
| `--mac` | macOS universal (x86_64 + aarch64 via lipo) | `clay-macos-universal` |
| `--linux` | Linux musl + Linux GUI+audio | `clay-linux-x86_64-musl`, `clay-linux-x86_64-gui` |
| `--windows` | Windows x86_64 via Windows VM (192.168.2.14) | `clay-windows-x86_64.exe` |

Examples:
- `/release 1.2.0` — Full release of v1.2.0, ask about Termux
- `/release` — Full release using current Cargo.toml version, ask about Termux
- `/release 1.2.0 --android` — Build and upload only the Android APK for v1.2.0
- `/release --mac` — Build and upload only macOS binary for current version
- `/release --windows` — Build and upload only the Windows binary via the Windows VM

### Step 2: Read Machine Details and Verify GitHub Auth

Read the file `.claude/skills/release/machines.md` for exact IPs, ports, users, paths, and build commands. Use it as your single source of truth for all machine details and build commands throughout this process.

Also verify GitHub authentication is working **before** spending time on builds:
```bash
gh auth status
```
If the token is invalid or missing, stop immediately and tell the user to run `gh auth login -h github.com` (suggest they type `! gh auth login -h github.com` to run it in-session). Do not proceed until auth is confirmed working.

### Step 3: Commit Pending Work, Then Update Version Numbers

**Before editing anything**, check whether the working tree already has uncommitted or unpushed work:
```bash
git status
git diff --stat
git log origin/master..HEAD --oneline   # unpushed commits, if any
```
If `git status` shows anything (staged, unstaged, or untracked), commit it now, before touching the version files, so it stays a separate, properly-described commit rather than getting bundled into the version bump. Running `/release` means the user wants everything currently in the working tree to ship — not just the version bump — so don't ask, just commit it:
```bash
git add <files>   # review `git status` first; never stage anything that looks like a
                   # secret (.env, credentials, private keys, tokens) even under this
                   # "commit everything" rule — that baseline safety check still applies
git commit -m "<describe what the diff actually contains>"
```
If `git log origin/master..HEAD` shows unpushed commits (from the commit just made, or already sitting there from a prior session), push now:
```bash
git push origin master
```
If the tree was already clean and fully pushed, skip this — there's nothing to do.

Then update the version string (without the `v` prefix) in these three files:
1. `Cargo.toml` line 3: `version = "X.Y.Z"`
2. `src/main.rs` line 26: `const VERSION: &str = "X.Y.Z";`
3. `android/app/build.gradle` line 14: `versionName "X.Y.Z"`

Also increment `versionCode` in `android/app/build.gradle` line 13 by 1 from its current value.

**Regenerate `Cargo.lock`** so the bump is committed together with its lock file:
```bash
cargo update -p clay --precise X.Y.Z 2>/dev/null || cargo check --quiet
```
(Any command that makes cargo rewrite the `clay` version line in `Cargo.lock` works.)

**Why this matters:** if the bump commit lands without the updated `Cargo.lock`, the first `cargo build` on *every* machine rewrites `Cargo.lock` locally. That leaves each remote's working tree dirty, and a dirty tree makes `git pull` abort — silently, since the build then proceeds against the **old checkout**. This has shipped stale binaries before. Never commit a version bump without its lock file.

### Step 4: Commit and Push

If any of the version files were actually modified in Step 3:
```bash
git add Cargo.toml Cargo.lock src/main.rs android/app/build.gradle
git commit -m "Bump version to vX.Y.Z"
git push origin master
```

If all files were already at the target version (nothing changed), skip the commit and push — the existing HEAD is already correct.

Wait for push to complete before proceeding. Record the resulting commit SHA:
```bash
EXPECTED_SHA=$(git rev-parse HEAD)
```
Every remote build must be verified to be at this SHA (see Step 6).

### Step 5: Build Local Targets

Skip targets not selected by a platform flag.

**Run local cargo builds ONE AT A TIME.** They are not independent, whatever the target
triple says: cargo takes an exclusive lock on the whole `target/` directory, so a second
build prints `Blocking waiting for file lock on build directory` and waits. That is merely
slow when you are watching it — but a *backgrounded* build that blocks can end without ever
running its remaining steps, and the shell still reports **exit 0**. In the v1.5.41 run the
Termux chain was started while the Linux GUI build was still going; only armv7 built, the
aarch64 binary and the APK stayed at the previous release's bytes, and nothing failed. Every
local target below shares one `target/` dir — musl, GUI, both Termux cross-builds, and the
APK (whose `buildNativeServer` task shells out to cargo).

Build in this order, each to completion before starting the next:

1. **Linux musl** — see machines.md for exact command
2. **Linux GUI+audio** — see machines.md for exact command
3. **Termux armv7 / aarch64 cross-builds** — see below
4. **Android APK** — see machines.md for build, align, and sign commands

**A chain of local builds must fail loudly.** Do not string them together with newlines in
one backgrounded shell and read only the tail: a step that dies mid-chain looks identical to
one that succeeded. Either run each as its own command, or use `set -e` and check the exit
code. After every local build, confirm the artifact it was supposed to produce actually
exists **and was written by this run**:

```bash
BUMP_TS=$(git log -1 --format=%ct "$EXPECTED_SHA")   # the version-bump commit
f=<artifact>
[ -e "$f" ] && [ "$(stat -c %Y "$f")" -ge "$BUMP_TS" ] \
  || echo "STALE: $f predates the bump commit — that build did not run"
```

Compare against the **bump commit**, not a fixed time window: an artifact older than the
commit it is supposed to be built from cannot contain this release's code, and a real release
spans well over half an hour, so any "built in the last N minutes" rule produces false alarms
on whichever target finished first. Note also that `find -newermt '-30 minutes'` does **not**
work here — `find` on this machine is `bfs`, which rejects GNU's relative timestamp format and
errors out rather than matching.

This is the check that caught the v1.5.41 near-miss. Step 7's marker grep is the backstop
and would also have caught it (the previous release's binary greps 0 for a string added in
this one), but it runs only at the very end, after every other target has been built — and
it degrades on a release that adds no new strings. An mtime check costs nothing, catches the
failure at the point it happens, and does not depend on choosing a good marker.

**Commit the freshly-built APK immediately after signing it**, before moving on to remote
builds:
```bash
git add android/clay-android.apk android/clay-android.apk.idsig
git commit -m "Update Android APK for vX.Y.Z release"
git push origin master
```
Do this even though it isn't required for `EXPECTED_SHA`/remote-sync purposes (no remote
build touches `android/`). Skipping it leaves the repo's tracked APK silently out of sync
with the binary actually uploaded to the GitHub release — the same class of staleness this
skill works hard to prevent for source code, just for a binary asset instead. It also
means the *next* `/release` run starts with an already-dirty `android/` in its Step 3
`git status` check, i.e. a stale leftover APK sitting in the tree is exactly the kind of
prior-run residue that caused `zipalign` to silently refuse to regenerate
`clay-android-aligned.apk` and sign old code (see the `-f` note in machines.md's APK
Signing section) — committing here closes that gap at the source rather than relying on
the Step 7 freshness check to catch it after the fact.

**Windows**: Built remotely on the Windows VM (see Step 6c). Do NOT cross-compile.
- Windows is never built as part of `--linux`, `--android`, or `--mac` partial releases.
- Fallback: if the VM is unreachable *and* `clay.exe` exists in the project root, use it as a hand-built fallback (copy to `/tmp/clay-windows-x86_64.exe`). If it doesn't exist either, warn and skip Windows.

**Termux armv7 (cross-compiled, no GUI)**: Built locally via the Android NDK — see machines.md.
- Run `./build-termux-armv7.sh`; output lands at `/tmp/clay-termux-armv7-32bit-nogui`.
- Unlike the Termux GUI build (Step 6c), this does not depend on the phone's reachability.
- Skip for `--android`, `--mac`, `--linux`, and `--windows` partial releases — Termux assets (all of
  them, aarch64 and armv7) are full-release only, consistent with Step 6c.

**Termux aarch64 (cross-compiled, no GUI)**: Built locally via the same Android NDK — see machines.md.
- Run `./build-termux-aarch64.sh`; output lands at `/tmp/clay-termux-aarch64-nogui`.
- Also does not depend on the phone's reachability (only the GUI variant still builds on-device,
  in Step 6c — the no-GUI aarch64 build used to be phone-built too, but was moved here since
  Termux's Bionic libc is ABI-compatible with a straight NDK cross-compile and this needs no
  Termux-specific GTK3/WebKit2GTK/X11 libraries).
- Skip for `--android`, `--mac`, `--linux`, and `--windows` partial releases — same rule as armv7.

If any build fails, read the error output, attempt to fix the code, commit and push the fix, then retry the failed build.

### Step 6: Build Remote Targets

Skip if platform flag is `--android` or `--linux`.

Before building on each remote machine, **probe reachability** to decide whether to attempt the build or skip it. Build these in sequence (one SSH session at a time).

#### Syncing a remote — REQUIRED before every remote build

**Never run a bare `git pull && cargo build` on a remote.** A remote's working tree is routinely dirty (cargo rewrites `Cargo.lock`, stray scratch files accumulate). When the tree is dirty, `git pull` prints `Please commit your changes or stash them before you merge. Aborting` — and if you chained it with `&&` while tailing only the last line, the abort is easy to miss and **the build silently proceeds against the old checkout**. That has shipped stale binaries.

Use this sequence on every **bash** remote (Mac, Termux — `<dir>` from machines.md):
```bash
cd <dir>
git stash push -- Cargo.lock          # harmless: cargo regenerates it
git pull
git rev-parse HEAD                    # MUST equal $EXPECTED_SHA from Step 4
```

**The Windows VM (192.168.2.14) runs `cmd.exe`, NOT bash.** The bash sequence above silently fails there — `;` is not a command separator in cmd, `2>/dev/null` is bash-only (cmd uses `2>nul`), and `test "$(...)"` doesn't exist — so `git pull` never runs and the build proceeds on the OLD checkout (this shipped a stale Windows binary in the v1.1.0 re-cut; the marker check below caught it). Use cmd.exe syntax instead:
```cmd
cd clay && git checkout -- Cargo.lock && git pull && git rev-parse HEAD
```
- `&&` (not `;`) chains commands in cmd and stops on the first failure, so a `git pull` abort halts the chain instead of being skipped past.
- `git checkout -- Cargo.lock` discards the locally-rewritten lock (it's committed, so this is safe) — simpler than `stash` and avoids any `2>/dev/null` redirect. Do NOT append `2>/dev/null`/`2>nul`; you want to SEE a pull error.
- Then compare the printed `git rev-parse HEAD` to `EXPECTED_SHA` yourself before running the build command.

If `git rev-parse HEAD` does not match `EXPECTED_SHA` (on any remote), **stop — do not build**. Investigate (another dirty file, a diverged branch, a detached HEAD) and resolve it before continuing. Do not use `git reset --hard`: it destroys uncommitted work you cannot see.

**Then verify the binary you copied back is actually the new code** — build logs lie when the checkout is stale. Pick a string unique to this release (a function name, a new setting, or the version) and grep the finished binary:
```bash
strings /tmp/<asset> | grep -c '<string-added-in-this-release>'   # must be > 0
```
A `0` means you built stale source: re-sync and rebuild. Do this for *every* asset before Step 8 — it is the only check that catches a silent stale build.

#### 6a. Mac (192.168.2.12)

Probe SSH reachability:
```bash
ssh -o ConnectTimeout=5 -o BatchMode=yes -o StrictHostKeyChecking=no user@192.168.2.12 exit
```
- Exit 0 → machine is up, proceed with build.
- Any other result (timeout, refused, host unreachable) → skip Mac build; mark as "Skipped (unreachable)" in summary.

If reachable:
1. Sync per the **Syncing a remote** procedure above (stash lock → pull → assert SHA)
2. Run the macOS universal build command from machines.md
3. `scp` the binary back to `/tmp/clay-macos-universal`, then verify it contains this release's code

#### 6b. Windows VM (192.168.2.14)

Skip if platform flag is `--android`, `--linux`, or `--mac`.

**Start the VM** (it is kept shut down when not in use):
```bash
VBoxManage startvm clay-win11 --type headless
```
- If this returns an error containing "already running", the VM is already up — continue.
- If VBoxManage itself is not found or returns a different error, skip Windows and mark as "Skipped (VirtualBox unavailable)".

**Wait for SSH** to become available (VM takes ~30s to boot):
```bash
for i in $(seq 1 24); do
  nc -z -w 3 192.168.2.14 22 && break
  sleep 5
done
```
After the loop, probe one final time:
```bash
ssh -o ConnectTimeout=5 -o BatchMode=yes -o StrictHostKeyChecking=no adrick@192.168.2.14 exit
```
- Exit 0 → SSH is up, proceed with build.
- Any other result → shut the VM back down. SSH is unreachable here, so the graceful route below is unavailable: use `VBoxManage controlvm clay-win11 acpipowerbutton` and poll for `poweroff`, falling back to `VBoxManage controlvm clay-win11 poweroff` only if it is still running after ~60s (see the hard-poweroff warning in machines.md — it corrupts the checkout). Then check for a fallback `clay.exe` in the project root (copy it to `/tmp/clay-windows-x86_64.exe` if present); otherwise mark Windows as "Skipped (VM did not come up)" in summary.

**Build (cmd.exe — this VM is NOT bash; see the cmd.exe note in "Syncing a remote"):**
1. Sync + build in one cmd.exe invocation, then read the printed SHA and confirm it equals `EXPECTED_SHA` before trusting the binary:
   ```cmd
   cd clay && git checkout -- Cargo.lock && git pull && git rev-parse HEAD && set RUSTFLAGS=-C target-feature=+crt-static && cargo build --release --features webview-gui,native-audio
   ```
   (`set VAR=val && cargo ...` in the same line is how cmd passes an env var to the build; that part was already correct in machines.md — the sync prefix is the new part.)
2. `scp` the binary back: `scp adrick@192.168.2.14:clay/target/release/clay.exe /tmp/clay-windows-x86_64.exe`
3. **Verify** it contains this release's code (`strings /tmp/clay-windows-x86_64.exe | grep -c '<release-marker>'` must be > 0). A `0` here means the cmd.exe sync didn't take and you built stale source — re-run step 1 and re-check the printed SHA.

**Shut the VM down gracefully** (always, whether the build succeeded or failed). A hard
`controlvm poweroff` discards Windows' recent disk writes and corrupts the VM's git index,
which silently ships a stale binary on a later run — see machines.md for the evidence:
```bash
ssh -o ConnectTimeout=10 -o BatchMode=yes -o StrictHostKeyChecking=no \
    adrick@192.168.2.14 "shutdown /s /t 0"
for i in $(seq 1 36); do
  st=$(VBoxManage showvminfo clay-win11 --machinereadable 2>/dev/null | grep '^VMState=' | cut -d'"' -f2)
  [ "$st" = "poweroff" ] && break
  sleep 5
done
```
If it is still running after that loop, try `VBoxManage controlvm clay-win11 acpipowerbutton`
and poll again; use `poweroff` only as a genuine last resort.

If a remote build fails:
- Read the error output
- If it's a code issue: fix the code locally, commit, push, start the VM again, then `git pull` on the VM and retry; shut down after retry
- If it's an environment issue (missing tool, etc.): shut the VM down, report to the user and ask how to proceed

#### 6c. Termux (192.168.2.50)

The Termux host (a phone) may be powered on but have its SSH server stopped. Use `nc` to probe the SSH port so you can report a more useful status:
```bash
nc -z -w 5 192.168.2.50 8022
```
- Exit 0 → SSH port is open, proceed with build.
- Exit non-zero with a quick "Connection refused" → host is up but SSH server is not running; skip and mark "Skipped (SSH not running)".
- Exit non-zero after timeout → host is unreachable; skip and mark "Skipped (unreachable)".

To distinguish refused vs timeout, capture the stderr or use a short timeout: a "Connection refused" arrives in under 1 s, while a timeout takes the full 5 s.

If reachable, build the Termux **GUI** binary (the only Termux target still built on-device —
the no-GUI aarch64 build is cross-compiled locally in Step 5 instead, since it needs no
Termux-specific GTK3/WebKit2GTK/X11 libraries; see machines.md's "Termux aarch64 binary
(cross-compiled, no GUI)" section for why):
1. Sync per the **Syncing a remote** procedure (stash lock → pull → assert SHA)
2. Run `./patches/apply-patches.sh` to patch tao/wry for Termux
3. Run the Termux aarch64 **GUI** build command from machines.md
4. `scp` the binary back to `/tmp/clay-termux-aarch64`, then verify it contains this release's code

If a remote build fails:
- Read the error output
- If it's a code issue: fix the code locally, commit, push, then `git pull` on the remote and retry
- If it's an environment issue (missing tool, etc.): report to the user and ask how to proceed

### Step 7: Collect Release Assets

Gather only the assets being released. Omit any remote target that was skipped due to being unreachable.

**Before uploading, verify every collected asset contains this release's code** (see "Syncing a remote" in Step 6). Grep each binary for a string unique to this release and require a non-zero count; for the APK, check the packaged asset instead:
```bash
strings /tmp/clay-linux-x86_64-musl | grep -c '<string-added-in-this-release>'
unzip -p android/clay-android.apk assets/web/app.js | grep -c '<string-added-in-this-release>'
```
Any asset returning `0` was built from stale source — do not upload it; re-sync that machine and rebuild. (A macOS universal binary reports roughly double the count: two architectures.)

**And check every asset's mtime in one go**, which catches a build that never ran at all —
including one whose shell reported success (see Step 5):

```bash
BUMP_TS=$(git log -1 --format=%ct "$EXPECTED_SHA")
for f in /tmp/clay-linux-x86_64-musl /tmp/clay-linux-x86_64-gui /tmp/clay-windows-x86_64.exe \
         android/clay-android.apk /tmp/clay-macos-universal \
         /tmp/clay-termux-aarch64-nogui /tmp/clay-termux-armv7-32bit-nogui; do
  if [ -e "$f" ] && [ "$(stat -c %Y "$f")" -ge "$BUMP_TS" ]; then st=OK; else st=STALE; fi
  printf '  %-42s %s\n' "$(basename "$f")" "$st"
done
```

Every asset must say OK. A STALE one is the signature of the v1.5.41 near-miss: the artifact
from the *previous* release is still sitting at that path, so nothing looks broken until you
compare timestamps. Skip any asset whose target was deliberately skipped. This check does not
depend on how good the release marker is, so run it even when the greps all pass.

**Also verify the Android APK bundles the standalone-mode native server** — `android.sh` already
asserts this before signing, but if the APK was produced some other way, check it here too:
```bash
unzip -l android/clay-android.apk | grep -c 'lib/arm64-v8a/libclay.so'
```
A `0` here means standalone/local-server mode is silently broken in this build (see machines.md's
Android APK section) — do not upload it.

For a full release:

| Source | Local Path | Asset Name |
|--------|-----------|------------|
| Linux musl | `target/x86_64-unknown-linux-musl/release/clay` | `clay-linux-x86_64-musl` |
| Linux GUI | `/tmp/clay-linux-x86_64-gui` | `clay-linux-x86_64-gui` |
| Windows (if reachable) | `/tmp/clay-windows-x86_64.exe` | `clay-windows-x86_64.exe` |
| Android | `android/clay-android.apk` | `clay-android.apk` |
| Mac | `/tmp/clay-macos-universal` | `clay-macos-universal` |
| Termux GUI (if reachable) | `/tmp/clay-termux-aarch64` | `clay-termux-aarch64` |
| Termux aarch64 no-GUI (cross-compiled locally) | `/tmp/clay-termux-aarch64-nogui` | `clay-termux-aarch64-nogui` |
| Termux armv7 no-GUI (cross-compiled locally) | `/tmp/clay-termux-armv7-32bit-nogui` | `clay-termux-armv7-32bit-nogui` |

For platform-limited releases, only collect the asset(s) listed in the platform table in Step 1.

**Note on Linux/Mac assets**: `gh release upload path#name` uses the source filename, not the label, as the asset name. Always copy binaries to a file whose name matches the asset name before uploading:
```bash
cp target/x86_64-unknown-linux-musl/release/clay /tmp/clay-linux-x86_64-musl
# Windows is already at /tmp/clay-windows-x86_64.exe (scp'd from VM, or copied from fallback clay.exe)
# Linux GUI and Mac are already copied to /tmp with the correct name
```

### Step 8: Create or Update GitHub Release

Check if the release tag already exists:
```bash
gh release view vX.Y.Z 2>/dev/null
```

**If the release exists:** Delete only the assets being replaced, then upload:
```bash
# For each asset name being uploaded that already exists on the release:
gh release delete-asset vX.Y.Z <asset-name> --yes

# Upload assets (using /tmp copies with correct filenames):
gh release upload vX.Y.Z /tmp/clay-linux-x86_64-musl#clay-linux-x86_64-musl /tmp/clay-termux-aarch64-nogui#clay-termux-aarch64-nogui /tmp/clay-termux-armv7-32bit-nogui#clay-termux-armv7-32bit-nogui ...
```

Do **not** delete assets for platforms not being released in this run.

**If the release doesn't exist (full release only):** Create it with whatever assets were collected (omit paths for any machine that was skipped/unreachable):
```bash
gh release create vX.Y.Z \
    /tmp/clay-linux-x86_64-musl#clay-linux-x86_64-musl \
    /tmp/clay-linux-x86_64-gui#clay-linux-x86_64-gui \
    [/tmp/clay-windows-x86_64.exe#clay-windows-x86_64.exe if Windows was reachable] \
    android/clay-android.apk#clay-android.apk \
    [/tmp/clay-macos-universal#clay-macos-universal if Mac was reachable] \
    [/tmp/clay-termux-aarch64#clay-termux-aarch64 if Termux was reachable] \
    /tmp/clay-termux-aarch64-nogui#clay-termux-aarch64-nogui \
    /tmp/clay-termux-armv7-32bit-nogui#clay-termux-armv7-32bit-nogui \
    --title "Clay vX.Y.Z" \
    --notes "Release vX.Y.Z"
```

Note: `clay-termux-aarch64-nogui` and `clay-termux-armv7-32bit-nogui` are not bracketed above —
both are built locally (Step 5), not gated on the phone's reachability, so they're always
included in a full release. Only `clay-termux-aarch64` (the GUI variant) still depends on the
phone.

### Step 9: Print Summary

Print a summary table showing:
- Each build target attempted, pass/fail status, and file size
- Skipped targets marked as "Skipped"
- The GitHub release URL
- Total number of assets uploaded

Example (full release, all machines reachable):
```
## Release vX.Y.Z Summary

| Target | Status | Size | Uploaded |
|--------|--------|------|----------|
| Linux musl x86_64 | PASS | 12.3 MB | Yes |
| Linux GUI x86_64 | PASS | 18.5 MB | Yes |
| Windows x86_64 (VM) | PASS | 14.1 MB | Yes |
| Android APK | PASS | 8.2 MB | Yes |
| macOS universal | PASS | 15.0 MB | Yes |
| Termux aarch64 (GUI) | PASS | 11.8 MB | Yes |
| Termux aarch64 (no GUI) | PASS | 6.4 MB | Yes |
| Termux armv7 (no GUI) | PASS | 6.1 MB | Yes |

Release: https://github.com/user/repo/releases/tag/vX.Y.Z
Assets uploaded: 8
```

Example (full release, Windows VM unreachable, Termux SSH not running, Mac unreachable):
```
## Release vX.Y.Z Summary

| Target | Status | Size | Uploaded |
|--------|--------|------|----------|
| Linux musl x86_64 | PASS | 12.3 MB | Yes |
| Linux GUI x86_64 | PASS | 18.5 MB | Yes |
| Windows x86_64 (VM) | Skipped (unreachable) | — | No |
| Android APK | PASS | 8.2 MB | Yes |
| macOS universal | Skipped (unreachable) | — | No |
| Termux aarch64 (GUI) | Skipped (SSH not running) | — | No |
| Termux aarch64 (no GUI) | PASS | 5.8 MB | Yes |
| Termux armv7 (no GUI) | PASS | 6.1 MB | Yes |

Release: https://github.com/user/repo/releases/tag/vX.Y.Z
Assets uploaded: 5
```

Note: `Termux aarch64 (no GUI)` and `Termux armv7 (no GUI)` are both built locally (Step 5), so
they're unaffected by the Termux phone's SSH reachability — they still build and upload even
when the GUI variant (still phone-built, Step 6c) is skipped.

Example (--android only):
```
## Release vX.Y.Z Summary (Android only)

| Target | Status | Size | Uploaded |
|--------|--------|------|----------|
| Android APK | PASS | 8.2 MB | Yes |
| Linux musl x86_64 | Skipped | — | No |
| Linux GUI x86_64 | Skipped | — | No |
| Windows x86_64 | Skipped | — | No |
| macOS universal | Skipped | — | No |
| Termux aarch64 (GUI) | Skipped | — | No |
| Termux aarch64 (no GUI) | Skipped | — | No |
| Termux armv7 (no GUI) | Skipped | — | No |

Release: https://github.com/user/repo/releases/tag/vX.Y.Z
Assets uploaded: 1
```

Example (--windows only):
```
## Release vX.Y.Z Summary (Windows only)

| Target | Status | Size | Uploaded |
|--------|--------|------|----------|
| Windows x86_64 (VM) | PASS | 14.1 MB | Yes |
| Linux musl x86_64 | Skipped | — | No |
| Linux GUI x86_64 | Skipped | — | No |
| Android APK | Skipped | — | No |
| macOS universal | Skipped | — | No |
| Termux aarch64 (GUI) | Skipped | — | No |
| Termux aarch64 (no GUI) | Skipped | — | No |
| Termux armv7 (no GUI) | Skipped | — | No |

Release: https://github.com/user/repo/releases/tag/vX.Y.Z
Assets uploaded: 1
```

## Error Handling

- **Build failure**: Read the error, fix the code, commit + push, git pull on remotes, retry
- **SSH failure**: Report to user, ask if they want to skip that target or retry
- **GitHub release failure**: Show the error and ask the user how to proceed
- **Signing failure**: Check if keystore exists, report error details to user
