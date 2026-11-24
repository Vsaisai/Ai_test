#!/usr/bin/env bash
set -euo pipefail
if [[ "${CI_COMMIT_TAG:-}" == mobile-v* ]]; then
  [[ "${CI_COMMIT_TAG#mobile-v}" == "$(cat mobile/VERSION)" ]] || exit 1
fi
apt-get update
apt-get install -y --no-install-recommends openjdk-17-jdk-headless curl unzip ca-certificates
export JAVA_HOME=/usr/lib/jvm/java-17-openjdk-amd64
export PATH="$JAVA_HOME/bin:$PATH"
mkdir -p "$ANDROID_HOME/cmdline-tools"
curl --fail --location --retry 3 https://dl.google.com/android/repository/commandlinetools-linux-11076708_latest.zip -o /tmp/android-tools.zip
unzip -q /tmp/android-tools.zip -d "$ANDROID_HOME/cmdline-tools"
mv "$ANDROID_HOME/cmdline-tools/cmdline-tools" "$ANDROID_HOME/cmdline-tools/latest"
sdkmanager="$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager"
# Capture sdkmanager's status, ignoring the expected SIGPIPE from yes.
set +o pipefail
yes | "$sdkmanager" --licenses >/dev/null
set -o pipefail
"$sdkmanager" 'platforms;android-35' 'build-tools;35.0.0' 'ndk;27.2.12479018'
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
cargo install cargo-ndk --locked
if [[ "${CI_COMMIT_TAG:-}" == mobile-v* ]]; then
  export VNSHELF_VERSION="${CI_COMMIT_TAG#mobile-v}"
fi
if [[ -n "${ANDROID_KEYSTORE_BASE64:-}" ]]; then
  : "${ANDROID_KEYSTORE_PASSWORD:?Missing signing password}"
  : "${ANDROID_KEY_ALIAS:?Missing signing alias}"
  : "${ANDROID_KEY_PASSWORD:?Missing key password}"
  umask 077
  export VNSHELF_KEYSTORE="$(mktemp)"
  trap 'rm -f "$VNSHELF_KEYSTORE"' EXIT
  printf '%s' "$ANDROID_KEYSTORE_BASE64" | base64 --decode > "$VNSHELF_KEYSTORE"
  export VNSHELF_KEYSTORE_PASSWORD="$ANDROID_KEYSTORE_PASSWORD"
  export VNSHELF_KEY_ALIAS="$ANDROID_KEY_ALIAS"
  export VNSHELF_KEY_PASSWORD="$ANDROID_KEY_PASSWORD"
else
  echo 'Using the repository debug key; this is a test APK, not a production-signed release.'
fi
chmod +x mobile/android/gradlew
bash mobile/build-apk.sh
