#!/usr/bin/env bash
# Clay Android-only release script
# Usage: ./android.sh [version]
#   version: e.g. "1.2.0" or "v1.2.0" (optional; defaults to Cargo.toml version)
export APKSIGNER=/home/adrick/Android/Sdk/build-tools/36.1.0/apksigner

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# ─── Colors ──────────────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; BOLD='\033[1m'; RESET='\033[0m'

log()  { echo -e "${CYAN}[android]${RESET} $*"; }
ok()   { echo -e "${GREEN}[  OK  ]${RESET} $*"; }
warn() { echo -e "${YELLOW}[ WARN ]${RESET} $*"; }
err()  { echo -e "${RED}[ERROR ]${RESET} $*" >&2; }
die()  { err "$*"; exit 1; }

# ─── Step 1: Determine version ───────────────────────────────────────────────
if [[ -n "${1:-}" ]]; then
    RAW_VERSION="${1}"
    BARE_VERSION="${RAW_VERSION#v}"
    BARE_VERSION="${BARE_VERSION// /-}"
    VERSION="v${BARE_VERSION}"
else
    BARE_VERSION=$(sed -n '3p' Cargo.toml | grep -oP '(?<=version = ")[^"]+')
    [[ -n "$BARE_VERSION" ]] || die "Could not read version from Cargo.toml"
    VERSION="v${BARE_VERSION}"
fi

log "Android release version: ${BOLD}${VERSION}${RESET}"

# ─── Step 2: Preflight checks ────────────────────────────────────────────────
log "Running preflight checks..."

command -v cargo  >/dev/null || die "'cargo' not found"
command -v git    >/dev/null || die "'git' not found"
command -v gh     >/dev/null || die "'gh' (GitHub CLI) not found"
command -v zipalign  >/dev/null || die "'zipalign' not found (Android build-tools)"
command -v $APKSIGNER >/dev/null || die "'apksigner' not found (Android build-tools)"

KEYSTORE_PASS_FILE="$HOME/.clay-keystore-pass"
[[ -f "$KEYSTORE_PASS_FILE" ]] || die "Keystore password file not found: $KEYSTORE_PASS_FILE"
KEYSTORE_FILE="android/clay-release.keystore"
[[ -f "$KEYSTORE_FILE" ]] || die "Keystore not found: $KEYSTORE_FILE"

gh auth status >/dev/null 2>&1 || die "Not authenticated with GitHub CLI. Run: gh auth login"

ok "Preflight checks passed."

# ─── Step 3: Commit any pre-existing changes ─────────────────────────────────
if ! git diff --quiet HEAD || ! git diff --cached --quiet; then
    log "Committing pre-existing changes..."
    git add -A
    git commit -m "Pre-release changes"
    git push origin master
    ok "Pre-existing changes pushed."
elif [[ $(git rev-list --count @{u}..HEAD 2>/dev/null || echo 0) -gt 0 ]]; then
    log "Pushing existing unpushed commits..."
    git push origin master
    ok "Unpushed commits pushed."
fi

# ─── Step 4: Update version numbers ─────────────────────────────────────────
log "Updating version numbers to ${BARE_VERSION}..."

sed -i "s/^version = \".*\"/version = \"${BARE_VERSION}\"/" Cargo.toml
grep -q "version = \"${BARE_VERSION}\"" Cargo.toml || die "Failed to update Cargo.toml"

sed -i "s/^const VERSION: &str = \".*\";/const VERSION: \&str = \"${BARE_VERSION}\";/" src/main.rs
grep -q "const VERSION: &str = \"${BARE_VERSION}\"" src/main.rs || die "Failed to update src/main.rs"

sed -i "s/versionName \".*\"/versionName \"${BARE_VERSION}\"/" android/app/build.gradle
grep -q "versionName \"${BARE_VERSION}\"" android/app/build.gradle || die "Failed to update versionName in build.gradle"

CURRENT_VERSION_CODE=$(grep -oP 'versionCode \K\d+' android/app/build.gradle)
[[ -n "$CURRENT_VERSION_CODE" ]] || die "Could not read versionCode from build.gradle"
NEW_VERSION_CODE=$(( CURRENT_VERSION_CODE + 1 ))
sed -i "s/versionCode ${CURRENT_VERSION_CODE}/versionCode ${NEW_VERSION_CODE}/" android/app/build.gradle
ok "versionCode: ${CURRENT_VERSION_CODE} → ${NEW_VERSION_CODE}"

# ─── Step 5: Commit and push ─────────────────────────────────────────────────
log "Committing version bump..."
git add Cargo.toml src/main.rs android/app/build.gradle
git commit -m "Bump version to ${VERSION}"
git push origin master
ok "Pushed to origin/master."

# ─── Step 6: Build Android APK ───────────────────────────────────────────────
log "Building Android APK..."
(cd android && ./gradlew assembleRelease) || die "Android Gradle build failed."

UNSIGNED_APK="android/app/build/outputs/apk/release/app-release-unsigned.apk"
[[ -f "$UNSIGNED_APK" ]] || die "Unsigned APK not found: $UNSIGNED_APK"

log "Signing Android APK..."
rm -f android/clay-android-aligned.apk android/clay-android.apk
zipalign -v -p 4 "$UNSIGNED_APK" android/clay-android-aligned.apk \
    || die "zipalign failed."
$APKSIGNER sign \
    --ks "$KEYSTORE_FILE" \
    --ks-pass "file:${KEYSTORE_PASS_FILE}" \
    --out android/clay-android.apk \
    android/clay-android-aligned.apk \
    || die "apksigner failed."
[[ -f "android/clay-android.apk" ]] || die "Signed APK not produced."
ok "Android APK signed."
APK_SIZE=$(du -sh "android/clay-android.apk" | cut -f1)

# ─── Step 7: Upload to GitHub ─────────────────────────────────────────────────
log "Uploading Android APK to GitHub release ${VERSION}..."

if gh release view "${VERSION}" >/dev/null 2>&1; then
    log "Release ${VERSION} exists — replacing clay-android.apk..."
    gh release delete-asset "${VERSION}" clay-android.apk --yes 2>/dev/null || true
    gh release upload "${VERSION}" "android/clay-android.apk#clay-android.apk" --clobber \
        || die "gh release upload failed."
else
    gh release create "${VERSION}" \
        "android/clay-android.apk#clay-android.apk" \
        --title "Clay ${VERSION}" \
        --notes "Release ${VERSION}" \
        || die "gh release create failed."
fi

RELEASE_URL=$(gh release view "${VERSION}" --json url -q .url)
ok "Uploaded to: ${RELEASE_URL}"

# ─── Step 8: Summary ─────────────────────────────────────────────────────────
echo ""
echo -e "${BOLD}## Android Release ${VERSION} Summary${RESET}"
echo ""
printf "%-20s %-8s %-10s %s\n" "Target" "Status" "Size" "Uploaded"
printf "%-20s %-8s %-10s %s\n" "------" "------" "----" "--------"
printf "%-20s %-8s %-10s %s\n" "Android APK" "PASS" "$APK_SIZE" "Yes"
echo ""
echo -e "Release: ${RELEASE_URL}"
