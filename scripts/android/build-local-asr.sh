#!/bin/bash
# Build the on-device dictation library (apps/android/local-asr) for Android.
#
#   scripts/android/build-local-asr.sh [out_dir]
#
# Parakeet Unified through parakeet-rs, behind JNI. It loads ONNX Runtime by path from the
# libonnxruntime.so the app's `onnxruntime-android` dependency ships, so nothing here links it.
# A workspace of its own (see its Cargo.toml). Without cargo-ndk / the NDK it is skipped: the
# app then has no on-device dictation and the car keeps using Android's recognizer.
# Run by the app's `buildLocalAsr` Gradle task (apps/android).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="${1:-$ROOT/target/android-core}"
mkdir -p "$OUT/jniLibs"
cd "$ROOT"

if command -v cargo-ndk >/dev/null && [[ -n "${ANDROID_NDK_HOME:-}" ]]; then
  ABIS=()
  for abi in ${ZERON_ANDROID_ABIS:-arm64-v8a x86_64}; do ABIS+=(-t "$abi"); done
  cargo ndk "${ABIS[@]}" -o "$OUT/jniLibs" \
    build --locked --manifest-path apps/android/local-asr/Cargo.toml --lib --release
  echo "jniLibs: $OUT/jniLibs"
else
  echo "note: cargo-ndk / ANDROID_NDK_HOME not found — skipped the local dictation library"
fi
