#!/bin/sh
# Optimized browser bundle plus precompressed siblings for the web gateway.
set -eu
cd "$(dirname "$0")"

trunk build --release

for file in dist/*.wasm dist/*.js; do
  gzip -9 -k -f "$file"
  zstd -19 -q -f "$file"
done

ls -l dist
