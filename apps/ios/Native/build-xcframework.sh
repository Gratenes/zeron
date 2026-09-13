#!/bin/sh
set -eu

HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO=$(CDPATH= cd -- "$HERE/../../.." && pwd)
OUTPUT="$REPO/target/tailcat/KratosTailcat.xcframework"

if [ "$(uname -s)" != Darwin ]; then
  echo "KratosTailcat.xcframework requires macOS and Xcode" >&2
  exit 1
fi
GOMOBILE=${GOMOBILE:-gomobile} "$REPO/scripts/build-tailcat.sh" xcframework
test -d "$OUTPUT"
echo "built $OUTPUT"