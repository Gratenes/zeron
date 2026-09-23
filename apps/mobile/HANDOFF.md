# Mobile Expo handoff

## Where to work

- Repository: `Gratenes/zeron`
- Branch: `feat/mobile-ui-parity`
- App: `apps/mobile`
- Product reference: the GPUI desktop/browser app. The SwiftUI app is a protocol reference.

## Current state

`App.tsx` is the live Expo application. It pairs through Tailcat, discovers the paired host engine, and uses authenticated ControlRpc for sessions, transcript, queue, projects, files, terminal, changes and history. The old Tailcat proof screen and local proxy display are gone. See `UI_PARITY_HANDOFF.md` for feature status and remaining parity work.

Android uses `connectivity/tailcat/netmon_android.go` for interface discovery because the platform denies Tailcat's usual `NETLINK_ROUTE` monitor. Android AAR builds require API 24 or later.

## Build and test

Use a supported Node version (20.19.4+, 22.13+, 24.3+, or 25+) and source the pinned native toolchain:

```sh
cd apps/mobile
source Native/env.sh
npm ci
npx tsc --noEmit
node --test src/utf8.test.mjs
./Native/build-aar.sh
cd android
./gradlew :app:assembleRelease
```

The APK is at `apps/mobile/android/app/build/outputs/apk/release/app-release.apk`. Expo Go cannot load the Tailcat native module. After installing a native build, `npx expo start --dev-client --tunnel` supports JavaScript iteration; Kotlin, Go, Gradle and native module edits require a rebuild.

## Important files

- `apps/mobile/App.tsx` — live app and ControlRpc subscriptions.
- `apps/mobile/src/connection.ts` — Tailcat session, authenticated RPC and reconnection.
- `apps/mobile/src/` — shell and feature panels.
- `apps/mobile/modules/my-module/android/src/main/java/ai/kratos/tailcat/KratosTailcatModule.kt` — Expo/Kotlin bridge.
- `connectivity/tailcat/adapter.go` — Go API exposed through `gomobile`.
- `apps/mobile/docs/gpui-parity.md` — GPUI scope inventory.

## Verification needed

The prior Tailcat connectivity proof was tested, but signed pairing and app RPC need a fresh end-to-end Android device test. Verify pairing, saved restore, session streams, run/queue, file and terminal operations, then Android foreground reconnection. Capture `adb logcat` if a native path fails.
