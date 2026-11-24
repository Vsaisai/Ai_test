#!/usr/bin/env bash
set -euo pipefail
apk="${1:?Pass the APK path}"
tools="${ANDROID_HOME:?}/build-tools/35.0.0"
"$tools/apksigner" verify "$apk"
metadata="$("$tools/aapt" dump badging "$apk")"
grep -q "^package: name='ru.vnshelf.app'" <<< "$metadata"
grep -q "^sdkVersion:'24'" <<< "$metadata"
grep -q "^targetSdkVersion:'35'" <<< "$metadata"
entries="$(unzip -Z -1 "$apk")"
grep -qx 'classes.dex' <<< "$entries"
for abi in arm64-v8a armeabi-v7a x86_64; do
  grep -Fxq "lib/$abi/libvnshelf.so" <<< "$entries"
done

if [[ "${CI_COMMIT_TAG:-}" == mobile-v* || "${CI_COMMIT_BRANCH:-}" == release/mobile-0.4.10 ]]; then
  version="$(cat mobile/VERSION)"
  code="$(awk -F. '{print (($1*100+$2)*100+$3)*100+$4}' <<< "$version")"
  grep -Fq "versionCode='$code' versionName='$version'" <<< "$metadata"
  signer="$("$tools/apksigner" verify --print-certs "$apk" | sed -n 's/^Signer #1 certificate SHA-256 digest: //p')"
  expected="${ANDROID_RELEASE_CERT_SHA256:-498c66c61edeea092a73575bbff512920b9f0f78f892fe659c03f94eea7388f3}"
  if [[ "$version" == 0.4.10 ]]; then expected="498c66c61edeea092a73575bbff512920b9f0f78f892fe659c03f94eea7388f3"; fi
  [[ "$expected" =~ ^[0-9a-f]{64}$ && "$signer" == "$expected" ]] || { echo 'APK signer mismatch'; exit 1; }
fi
