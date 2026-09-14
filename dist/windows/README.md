# Windows distribution

`zeron.ico` contains 16, 24, 32, 48, 64, 128, and 256 pixel PNG frames
converted from the existing `../macos/icon-1024.png` artwork with bicubic
resampling. It preserves the artwork's transparency.

`apps/zeron/build.rs` compiles `zeron.rc` into the Windows executable for
both debug and release builds. Resource ID 1 is required by GPUI's Windows
icon loader.

The supported portable installation is the versioned ZIP. It contains
`zeron.exe`, the pinned `kratos-tailcat.exe` adapter, and
`zeron-update.json`. Updates download and checksum that complete ZIP, reject
unexpected or unsafe archive entries, validate the adapter's package-bound
SHA-256, and replace the app, adapter, and configuration as one helper-owned
transaction after the app exits and releases package files. The versioned
standalone EXE is a legacy app-only release artifact, is not a complete
installation, and is never selected by the updater.
