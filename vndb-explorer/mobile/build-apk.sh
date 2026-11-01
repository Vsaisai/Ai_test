#!/usr/bin/env bash
# Build the Android APK: Rust library for every ABI (cargo-ndk) + Gradle shell.
# Needs: ANDROID_HOME (SDK with platform 35), ANDROID_NDK_HOME, cargo-ndk, Rust android targets, JDK 17+.
#   ABIS="arm64-v8a" ./mobile/build-apk.sh        # faster, phones only
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
: "${ANDROID_NDK_HOME:?set ANDROID_NDK_HOME to the NDK directory}"
ABIS="${ABIS:-arm64-v8a armeabi-v7a x86_64}"
VERSION="${VNSHELF_VERSION:-$(tr -d ' \n' < "$ROOT/mobile/VERSION")}"
export VNSHELF_VERSION="$VERSION"

targets=()
for abi in $ABIS; do targets+=(-t "$abi"); done

cd "$ROOT"
rm -rf mobile/android/app/src/main/jniLibs
cargo ndk "${targets[@]}" -P 24 -o mobile/android/app/src/main/jniLibs build --release -p vnshelf-mobile

cd mobile/android
./gradlew --no-daemon --console=plain assembleRelease

mkdir -p "$ROOT/target/apk"
OUT="$ROOT/target/apk/VNShelf-$VERSION.apk"
cp app/build/outputs/apk/release/app-release.apk "$OUT"
echo "APK: $OUT"
