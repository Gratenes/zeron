# Mobile Expo handoff

## Where to work

- Repository: `Gratenes/zeron`
- Branch: `feat/mobile-expo`
- App: `apps/mobile`
- Product reference: the GPUI desktop/browser app. The older SwiftUI app is a
  protocol reference, not the feature or design target.

This branch is isolated from the active web worktree. Do not use it for web UI
changes.

## Current state

The app is deliberately a Tailcat proof screen, not the final mobile UI. It
accepts a `kratos-pair:` payload, starts the native Tailcat client, and displays
the local proxy URL. The GPUI-style shell is in earlier history but is not the
current `App.tsx`.

Tailcat is the current gate: do not begin parity work until an Android device
can start the native client and reach the paired peer.

The Android route-monitor failure has a reproducible workaround:

- Android apps cannot open the `NETLINK_ROUTE` monitor Tailcat normally uses.
- `connectivity/tailcat/android-netmon.patch` makes Tailscale use a static
  monitor on Android while retaining the event bus required by `magicsock`.
- `connectivity/tailcat/build.sh` applies that patch only for Android AAR
  builds. It is tied to the `tailscale.com` version pinned in that script.

The prior native crash was a nil event bus in the first static-monitor attempt.
The current patch preserves the bus. It still needs an end-to-end device retry.

## Build and test

```sh
cd apps/mobile
source Native/env.sh
npm install
npx tsc --noEmit
./Native/build-aar.sh
cd android
./gradlew :app:assembleRelease
```

The release APK is at:

```text
apps/mobile/android/app/build/outputs/apk/release/app-release.apk
```

Use the release variant for the Tailcat proof. It embeds JavaScript and avoids
the Expo Dev Client lifecycle crash (`App react context shouldn't be created
before`). Expo Go cannot load the custom native Tailcat module.

For development builds, run `npx expo start --dev-client --tunnel` after the
native app is installed. JavaScript edits hot-reload; any Kotlin, Go, Gradle, or
native module change requires rebuilding and reinstalling the APK.

## Important files

- `apps/mobile/App.tsx` — temporary Tailcat proof UI.
- `apps/mobile/modules/my-module/android/src/main/java/ai/kratos/tailcat/KratosTailcatModule.kt` — Expo/Kotlin bridge.
- `connectivity/tailcat/adapter.go` — Go API exposed through `gomobile`.
- `connectivity/tailcat/build.sh` — native CLI/AAR build and Android patch step.
- `apps/mobile/docs/gpui-parity.md` — intended GPUI surface inventory.

## Next steps

1. Install the latest release APK and retry one fresh connect payload.
2. If it fails, capture `adb logcat`/the Android tombstone before changing code.
3. Once Tailcat connects, replace the probe `App.tsx` with the real app shell
   and implement GPUI parity by vertical slice: pairing, sessions, composer,
   streaming output, then secondary panels/settings.
4. Keep Android primary; check iOS and web as shared React Native surfaces,
   without treating the existing iOS client as the UI target.

## Known test note

`go test -run '^TestLocalDERPTwoPeerProxy$'` passes. The full Tailcat suite has
an existing flaky recovery assertion that sometimes observes 26 requests where
it expects 25; investigate it separately from the Android monitor work.
